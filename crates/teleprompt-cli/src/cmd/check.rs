use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use teleprompt_cache::VoiceCache;
use teleprompt_compile::{compile, CompileOutput, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Item};
use teleprompt_core::{Diagnostic, Diagnostics};
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::{VoiceBackend, VoiceRegistry};
use teleprompt_voice_null::WpmEstimator;

use crate::project::Project;

/// Where a project's synthesis cache lives. Shared between `compile_script`
/// (which only ever reads it) and `dub` (which populates it), so the two
/// can never drift onto different directories and silently stop agreeing
/// about what is warm.
pub fn cache_root(project: &Project) -> PathBuf {
    project.root.join(".teleprompt").join("cache")
}

/// Shared front half of every command: read, parse, identify, resolve, compile.
/// Has no side effects — it never writes to disk.
///
/// Returns the resolved backend alongside the compilation because the two
/// must be the same one: `cache_key` is computed from `backend.id()` and
/// `capabilities().version`, so a caller that synthesizes with a *different*
/// backend writes that backend's audio into the cache under this one's key.
/// `dub` is the only caller that reads it.
///
/// It is returned separately rather than being a field on `CompileOutput`
/// because `CompileOutput` is what `check`, `plan`, and `diff` receive: a
/// backend hanging off it would put `synthesize` one `.` away from the inner
/// loop, which is the thing `VoiceContext` exists to make impossible.
pub fn compile_script(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<(CompileOutput, Arc<dyn VoiceBackend>), Vec<String>> {
    compile_script_with(&crate::voice::registry(), project, script, locale)
}

/// [`compile_script`] against a caller-supplied registry.
///
/// The seam exists so a test can register a backend this build does not
/// ship and watch the whole path — key, synthesis, cache, manifest — follow
/// it. That is the claim spec §4.1 makes ("adding a backend is one line and
/// one new crate"), and it is not a claim a workspace with exactly one
/// backend registered can otherwise check.
pub fn compile_script_with(
    registry: &VoiceRegistry,
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<(CompileOutput, Arc<dyn VoiceBackend>), Vec<String>> {
    let name = script
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| script.display().to_string());
    let display = script.display().to_string();

    let src =
        std::fs::read_to_string(script).map_err(|e| vec![format!("cannot read {display}: {e}")])?;

    let mut parsed = parse_script(&src).map_err(|d| render(&d, &display))?;

    let id_diags = assign_ids(&mut parsed);
    if id_diags.iter().any(|d| d.is_error()) {
        return Err(render(&teleprompt_core::Diagnostics(id_diags), &display));
    }

    let program = resolve(
        &parsed,
        &name,
        locale,
        &project.config,
        &PartialConfig::default(),
    )
    .map_err(|d| render(&d, &display))?;

    // The *resolved* backend, not just a name: a script's own front matter
    // can override `voice.backend`, and this is what actually produces the
    // segment's audio, so `cache_key` and `backend_version` both need to
    // come from it rather than from the raw string.
    let backend =
        crate::voice::resolve(registry, &program.config.voice.backend).map_err(|e| vec![e])?;

    // Delivery A supports exactly one backend per compile: `VoiceContext`
    // carries a single `backend_id` for the whole program. A narration item
    // can resolve to a different backend than the program's overall
    // resolved backend two ways: a segment attribute set it, or a
    // chapter's front matter did. `Item::Narration` does not retain which
    // layer supplied the value — `config` is already the fully merged
    // result — so the diagnostic describes the *effect* ("this segment
    // resolves to a backend other than the one in use") rather than
    // guessing which layer is at fault and telling the author to remove an
    // attribute that, for a chapter-level override, was never written.
    //
    // Grouped by the offending value and reported once per group, naming
    // every affected segment, so a chapter of twelve paragraphs under one
    // stray `voice: { backend: ... }` produces one error, not twelve.
    let mut offenders: std::collections::BTreeMap<
        String,
        Vec<(String, teleprompt_core::SourceSpan)>,
    > = std::collections::BTreeMap::new();
    for item in &program.items {
        if let Item::Narration {
            id, config, span, ..
        } = item
        {
            if config.voice.backend != program.config.voice.backend {
                offenders
                    .entry(config.voice.backend.clone())
                    .or_default()
                    .push((id.clone(), *span));
            }
        }
    }
    if !offenders.is_empty() {
        let diags: Vec<Diagnostic> = offenders
            .into_iter()
            .map(|(backend, segments)| {
                let span = segments[0].1;
                let names = segments
                    .iter()
                    .map(|(id, _)| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let (noun, verb) = if segments.len() == 1 {
                    ("segment", "resolves")
                } else {
                    ("segments", "resolve")
                };
                Diagnostic::error(format!(
                    "{noun} {names} {verb} to voice backend `{backend}`, but this compile uses \
                     `{}`",
                    program.config.voice.backend
                ))
                .at(span)
                .with_help(
                    "per-segment and per-chapter voice backends are not supported yet; set \
                     voice.backend at the project or script front-matter level instead",
                )
            })
            .collect();
        return Err(render(&Diagnostics(diags), &display));
    }

    // `script_dir`, not `script.parent()`: a bare `demo.md` has
    // `Some("")` for a parent, which is not the current directory.
    let base_dir = crate::project::script_dir(script);

    let cache = VoiceCache::new(cache_root(project));
    let estimator = WpmEstimator::default();
    let capabilities = backend.capabilities();
    let ctx = VoiceContext {
        backend_id: backend.id(),
        backend_version: &capabilities.version,
        cache: &cache,
        estimator: &estimator,
    };

    let out = compile(
        &program,
        &SceneRegistry::with_builtins(),
        &ctx,
        base_dir,
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|d| render(&d, &display))?;

    Ok((out, backend))
}

/// Returns warnings on success, rendered errors on failure. `check` is
/// `compile_script` minus the timeline: parse, validate, compile — but the
/// caller only ever looks at whether it succeeded and what it has to say.
pub fn run_check(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<Vec<String>, Vec<String>> {
    compile_script(project, script, locale).map(|(out, _)| out.warnings)
}

fn render(d: &teleprompt_core::Diagnostics, file: &str) -> Vec<String> {
    d.0.iter().map(|x| x.render(file)).collect()
}

/// The stable, typed shape of `check`'s `--format json` output. Mirrors
/// `DoctorReport`'s and `NewReport`'s pattern rather than building JSON
/// inline in `main.rs`: `ok` says which of `warnings`/`errors` is
/// meaningful, but both fields are always present so consumers never have
/// to branch on a missing key.
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
