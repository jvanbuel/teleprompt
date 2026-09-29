use std::path::{Path, PathBuf};
use std::sync::Arc;
use teleprompt_core::program::Program;
use teleprompt_voice::takes::Takes;

use serde::Serialize;
use teleprompt_cache::VoiceCache;
use teleprompt_compile::{compile, CompileOutput, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Element};
use teleprompt_core::translation::Translation;
use teleprompt_core::{Diagnostic, Diagnostics};
use teleprompt_voice::VoiceBackend;
use teleprompt_voice::WpmEstimator;

use crate::project::Project;
use crate::voice::Backends;

/// Shared front half of every command: read, parse, identify, resolve,
/// compile. Writes nothing.
///
/// Backends are built from the project's `backends:` settings before the
/// script is read, so a script's front-matter `backends:` cannot reach them
/// (it gets a warning instead). The resolved backend is returned so a
/// synthesizing caller uses the one the cache keys were computed from. It
/// is kept off `CompileOutput`, which `check`, `plan` and `plan --check` receive,
/// to hold docs/design.md#async-boundary.
pub fn compile_script(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<(CompileOutput, Arc<dyn VoiceBackend>), Vec<String>> {
    compile_script_with(&backends_of(project), project, script, locale)
}

/// The project's backends, built once from its settings and told which file
/// those settings came from so a diagnostic about them can point at it.
pub(crate) fn backends_of(project: &Project) -> Backends {
    let settings = project.config.backends.clone().unwrap_or_default();
    crate::voice::backends_for(&settings, &project.config_path().display().to_string())
}

/// [`compile_script`] against caller-supplied backends.
///
/// The seam exists so a test can register a backend this build does not ship
/// and watch the whole path — key, synthesis, cache, manifest — follow it
/// (docs/design.md#crates), which a workspace with one real backend could not
/// otherwise check.
pub(crate) fn compile_script_with(
    backends: &Backends,
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<(CompileOutput, Arc<dyn VoiceBackend>), Vec<String>> {
    // Before the script is read: an unknown `backends:` key is wrong for
    // every script. Here rather than in `compile_script` so that `dub` and
    // the preview, which call this directly, cannot reach a server with it.
    let config_diags = backends.diagnostics();
    if !config_diags.is_empty() {
        return Err(render(
            &Diagnostics(config_diags),
            &script.display().to_string(),
        ));
    }

    let name = script
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| script.display().to_string());
    let display = script.display().to_string();

    let src =
        std::fs::read_to_string(script).map_err(|e| vec![format!("cannot read {display}: {e}")])?;

    let parsed = parse_script(&src).map_err(|d| render(&d, &display))?;

    let mut program = resolve(
        &parsed,
        &name,
        locale,
        &project.config,
        &PartialConfig::default(),
    )
    .map_err(|d| render(&d, &display))?;
    let translation_warnings = translate(&mut program, script, &display)?;

    // `resolve` merges front-matter `backends:`, but the backends were built
    // from the project's alone, so an override would silently do nothing. A
    // warning, not an error: the audio still comes from the project's
    // settings, which are correct, just not the script's variant of them.
    let base_backends = project.config.backends.clone().unwrap_or_default();
    let backend_override_diags =
        backend_override_warnings(&base_backends, &program.config.backends);

    // After front matter has had its say on `voice.backend`. Unusable
    // settings fail here only when this is the backend selected (see
    // `Backends`).
    let backend = backends
        .resolve(&program.config.voice.backend)
        .map_err(|d| render(&Diagnostics(vec![d]), &display))?;

    // One backend per compile: `VoiceContext` carries a single `backend_id`.
    // A line attribute or a chapter block can still pick another, and the
    // merged config no longer says which, so the error names the effect,
    // not the layer. One error per backend, naming every line, so a chapter
    // under one stray `voice.backend` is not twelve errors.
    let mut offenders: std::collections::BTreeMap<
        String,
        Vec<(String, teleprompt_core::SourceSpan)>,
    > = std::collections::BTreeMap::new();
    for item in &program.elements {
        if let Element::Narration {
            id, config, span, ..
        } = item
        {
            if config.voice.backend != program.config.voice.backend {
                offenders
                    .entry(config.voice.backend.clone())
                    .or_default()
                    .push((id.to_string(), *span));
            }
        }
    }
    if !offenders.is_empty() {
        let diags: Vec<Diagnostic> = offenders
            .into_iter()
            .map(|(backend, lines)| {
                let shot = lines[0].1;
                let names = lines
                    .iter()
                    .map(|(id, _)| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let (noun, verb) = if lines.len() == 1 {
                    ("line", "resolves")
                } else {
                    ("lines", "resolve")
                };
                Diagnostic::error(format!(
                    "{noun} {names} {verb} to voice backend `{backend}`, but this compile uses \
                     `{}`",
                    program.config.voice.backend
                ))
                .at(shot)
                .with_help(
                    "per-line and per-chapter voice backends are not supported yet; set \
                     voice.backend at the project or script front-matter level instead",
                )
            })
            .collect();
        return Err(render(&Diagnostics(diags), &display));
    }

    // `script_dir`, not `script.parent()`: a bare `demo.md` has
    // `Some("")` for a parent, which is not the current directory.
    let base_dir = crate::project::script_dir(script);

    let mut out = compile_with_voice(project, &program, &*backend, base_dir, &display)?;

    // The bare message, not `render()`'s output: like every other warning
    // here it is a plain sentence, and callers add their own framing.
    out.warnings
        .extend(backend_override_diags.into_iter().map(|d| d.message));
    out.warnings.extend(translation_warnings);

    Ok((out, backend))
}

/// For a locale other than the script's own, puts the translation beside
/// the script (`tour.nl.yaml` for `tour.md`) in place of its English,
/// returning what is out of date in it.
fn translate(
    program: &mut Program,
    script: &Path,
    display: &str,
) -> Result<Vec<String>, Vec<String>> {
    let locale = program.locale.clone();
    if locale == program.config.locales.source {
        return Ok(Vec::new());
    }
    let path = translation_path(script, &locale);
    let yaml = std::fs::read_to_string(&path).map_err(|e| {
        let d = Diagnostic::error(format!(
            "there is no `{locale}` translation at {}: {e}",
            path.display()
        ))
        .with_help(format!(
            "run `teleprompt translate {display} --to {locale}`"
        ));
        render(&Diagnostics(vec![d]), display)
    })?;
    let translation =
        Translation::from_yaml(&yaml).map_err(|e| vec![format!("{}: {e}", path.display())])?;
    let diags = teleprompt_core::translation::apply(program, &translation);
    if diags.iter().any(Diagnostic::is_error) {
        return Err(render(&Diagnostics(diags), display));
    }
    Ok(diags.into_iter().map(|d| d.message).collect())
}

/// Where `script`'s `locale` translation is kept.
pub fn translation_path(script: &Path, locale: &str) -> PathBuf {
    script.with_extension(format!("{locale}.yaml"))
}

/// Compiles `program` against the project's voice cache and takes, naming
/// the lines still synthesized once there are takes.
fn compile_with_voice(
    project: &Project,
    program: &Program,
    backend: &dyn VoiceBackend,
    base_dir: &Path,
    display: &str,
) -> Result<CompileOutput, Vec<String>> {
    let cache = VoiceCache::new(project.caches().root);
    let estimator = WpmEstimator::default();
    let capabilities = backend.capabilities();
    let takes = Takes::load(&project.takes_dir()).map_err(|e| vec![e.to_string()])?;
    let ctx = VoiceContext {
        backend_id: backend.id(),
        backend_version: &capabilities.version,
        cache: &cache,
        estimator: &estimator,
        takes: &takes,
    };

    // Not `SceneRegistry::with_builtins()`, which holds only the mock.
    let mut out = compile(
        program,
        &crate::scene::scenes(),
        &ctx,
        base_dir,
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|d| render(&d, display))?;
    out.warnings.extend(unrecorded(&takes, &out));
    Ok(out)
}

/// Once a project records its narration, which lines it still synthesizes:
/// never recorded, or edited since.
fn unrecorded(takes: &Takes, out: &CompileOutput) -> Option<String> {
    if takes.is_empty() {
        return None;
    }
    let ids: Vec<&str> = out
        .narration
        .iter()
        .filter(|d| d.take.is_none())
        .map(|d| d.line_id.as_str())
        .collect();
    (!ids.is_empty()).then(|| {
        format!(
            "{} of {} lines have no current take and are synthesized: {}",
            ids.len(),
            out.narration.len(),
            ids.join(", ")
        )
    })
}

/// One warning per backend id whose `merged` entry (after `resolve`) differs
/// from `base`, the project settings the backends were built from.
fn backend_override_warnings(
    base: &std::collections::BTreeMap<String, serde_yaml::Value>,
    merged: &std::collections::BTreeMap<String, serde_yaml::Value>,
) -> Vec<Diagnostic> {
    merged
        .iter()
        .filter(|(id, value)| base.get(*id) != Some(*value))
        .map(|(id, value)| {
            let keys = differing_backend_keys(base.get(id), value);
            let which = if keys.is_empty() {
                String::new()
            } else {
                format!(" (`{}`)", keys.join("`, `"))
            };
            Diagnostic::warning(format!(
                "this script's front matter sets `backends.{id}`{which}, but backend settings \
                 are resolved once per project before any script is parsed, so the override has \
                 no effect here; set it in `teleprompt.toml` under `backends.{id}` instead"
            ))
        })
        .collect()
}

/// The mapping keys `after` sets or changes relative to `before`. Empty
/// unless both are mappings: otherwise there is nothing finer to name than
/// the backend itself.
fn differing_backend_keys(
    before: Option<&serde_yaml::Value>,
    after: &serde_yaml::Value,
) -> Vec<String> {
    let (Some(before), Some(after)) = (before.and_then(|v| v.as_mapping()), after.as_mapping())
    else {
        return Vec::new();
    };
    after
        .iter()
        .filter(|(k, v)| before.get(*k) != Some(*v))
        .filter_map(|(k, _)| k.as_str().map(str::to_string))
        .collect()
}

/// Returns warnings on success, rendered errors on failure.
pub fn run_check(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<Vec<String>, Vec<String>> {
    let mut warnings = compile_script(project, script, locale)?.0.warnings;
    // What is hard to say aloud, in the language the lines are said in.
    let program = resolved(project, script, locale)?;
    let display = script.display().to_string();
    warnings.extend(teleprompt_core::lint::lint(&program).iter().map(|d| {
        d.render(&display)
            .trim_start_matches("warning: ")
            .to_string()
    }));
    Ok(warnings)
}

/// `script` resolved in its own language, for reading its narration rather
/// than compiling it.
pub(crate) fn source_program(project: &Project, script: &Path) -> Result<Program, Vec<String>> {
    resolved(project, script, &source_locale(project))
}

/// The language the project's scripts are written in: the locale a command
/// compiles for when none is given.
pub fn source_locale(project: &Project) -> String {
    teleprompt_core::config::Config::merged(std::slice::from_ref(&project.config))
        .locales
        .source
}

/// `script` resolved for `locale`, untranslated: its configuration as that
/// locale sees it.
pub(crate) fn resolved(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<Program, Vec<String>> {
    let display = script.display().to_string();
    let name = script
        .file_name()
        .map_or_else(|| display.clone(), |s| s.to_string_lossy().to_string());
    let src =
        std::fs::read_to_string(script).map_err(|e| vec![format!("cannot read {display}: {e}")])?;
    let parsed = parse_script(&src).map_err(|d| render(&d, &display))?;
    resolve(
        &parsed,
        &name,
        locale,
        &project.config,
        &PartialConfig::default(),
    )
    .map_err(|d| render(&d, &display))
}

fn render(d: &teleprompt_core::Diagnostics, file: &str) -> Vec<String> {
    d.0.iter().map(|x| x.render(file)).collect()
}

/// `check`'s `--format json` output. Both lists are always present, so a
/// consumer never branches on a missing key.
#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub ok: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_report_serializes_ok_and_both_lists() {
        let report = CheckReport {
            ok: true,
            warnings: vec!["bare wait".to_string()],
            errors: vec![],
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"ok": true, "warnings": ["bare wait"], "errors": []})
        );
    }
}
