//! Compiling a script in its project: read, parse, resolve with the
//! project's configuration and the script's translation, and compile with
//! the project's voice backends. Every command that reads a script starts
//! here; `check` is the one that stops.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use teleprompt_core::program::Program;
use teleprompt_voice::takes::Takes;

use teleprompt_compile::{compile, CompileOutput, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Element};
use teleprompt_core::translation::Translation;
use teleprompt_core::{Diagnostic, Diagnostics};
use teleprompt_voice::cache::VoiceCache;
use teleprompt_voice::VoiceBackend;
use teleprompt_voice::WpmEstimator;

use super::Project;
use crate::voice::Backends;

impl Project {
    /// Shared front half of every command: read, parse, identify, resolve,
    /// compile. Writes nothing.
    ///
    /// Backends are built from the project's `backends:` settings before the
    /// script is read, so a script's front-matter `backends:` cannot reach them
    /// (it gets a warning instead). The resolved backend is returned so a
    /// synthesizing caller uses the one the cache keys were computed from. It
    /// is kept off `CompileOutput`, which `check`, `plan` and `plan --check` receive,
    /// to hold docs/design.md#async-boundary.
    pub fn compile(
        &self,
        script: &Path,
        locale: &str,
    ) -> Result<(CompileOutput, Arc<dyn VoiceBackend>), Vec<String>> {
        self.compile_with(&self.backends(), script, locale)
    }

    /// The project's backends, built once from its settings and told which file
    /// those settings came from so a diagnostic about them can point at it.
    pub fn backends(&self) -> Backends {
        let settings = self.config.backends.clone().unwrap_or_default();
        crate::voice::backends_for(&settings, &self.config_path().display().to_string())
    }

    /// [`Project::compile`] against caller-supplied backends.
    ///
    /// The seam exists so a test can register a backend this build does not ship
    /// and watch the whole path — key, synthesis, cache, manifest — follow it
    /// (docs/design.md#crates), which a workspace with one real backend could not
    /// otherwise check.
    pub(crate) fn compile_with(
        &self,
        backends: &Backends,
        script: &Path,
        locale: &str,
    ) -> Result<(CompileOutput, Arc<dyn VoiceBackend>), Vec<String>> {
        self.compile_file(backends, script, locale)
            .map(|c| (c.output, c.backend))
    }

    /// [`Project::compile_source`] on `script` as saved, its problems
    /// rendered.
    pub fn compile_file(
        &self,
        backends: &Backends,
        script: &Path,
        locale: &str,
    ) -> Result<Compiled, Vec<String>> {
        let display = script.display().to_string();
        let src = std::fs::read_to_string(script)
            .map_err(|e| vec![format!("cannot read {display}: {e}")])?;
        self.compile_source(backends, script, &src, locale)
            .map_err(|d| render(&d, &display))
    }
}

/// A script compiled: what it compiles to, the narrator's backend, and the
/// program it was compiled from.
pub struct Compiled {
    pub output: CompileOutput,
    pub backend: Arc<dyn VoiceBackend>,
    pub program: Program,
}

impl Project {
    /// [`Project::compile_with`] on `src`, the text of `script` as it may be in
    /// an editor, not yet saved; its problems as diagnostics, not text.
    pub fn compile_source(
        &self,
        backends: &Backends,
        script: &Path,
        src: &str,
        locale: &str,
    ) -> Result<Compiled, Diagnostics> {
        let project = self;
        // Before the script is read: an unknown `backends:` key is wrong for
        // every script. Here rather than in `compile` so that `dub` and
        // the prompter, which call this directly, cannot reach a server with it.
        let config_diags = backends.diagnostics();
        if !config_diags.is_empty() {
            return Err(Diagnostics(config_diags));
        }
        let name = script
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| script.display().to_string());
        let display = script.display().to_string();
        let parsed = parse_script(src)?;
        let mut program = resolve(
            &parsed,
            &name,
            locale,
            &project.config,
            &PartialConfig::default(),
        )?;
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
            .map_err(|d| Diagnostics(vec![d]))?;
        // Every other backend a line is spoken by, as a cast's speakers are.
        let others = other_backends(backends, &program, backend.id()).map_err(Diagnostics)?;

        // `script_dir`, not `script.parent()`: a bare `demo.md` has
        // `Some("")` for a parent, which is not the current directory.
        let base_dir = crate::project::script_dir(script);
        let mut output = compile_with_voice(project, &program, &*backend, others, base_dir)?;

        // The bare message, not `render()`'s output: like every other warning
        // here it is a plain sentence, and callers add their own framing.
        output
            .warnings
            .extend(backend_override_diags.into_iter().map(|d| d.message));
        output.warnings.extend(translation_warnings);
        Ok(Compiled {
            output,
            backend,
            program,
        })
    }
}

/// The version of each backend a line is spoken by other than `main`, by
/// id; or, for each that does not exist or cannot be used, an error naming
/// every line that asks for it.
fn other_backends(
    backends: &Backends,
    program: &Program,
    main: &str,
) -> Result<std::collections::BTreeMap<String, String>, Vec<Diagnostic>> {
    let mut wanted: std::collections::BTreeMap<String, Vec<(String, teleprompt_core::SourceSpan)>> =
        std::collections::BTreeMap::new();
    for item in &program.elements {
        if let Element::Narration {
            id, config, span, ..
        } = item
        {
            if config.voice.backend != main {
                wanted
                    .entry(config.voice.backend.clone())
                    .or_default()
                    .push((id.to_string(), *span));
            }
        }
    }
    let mut versions = std::collections::BTreeMap::new();
    let mut errors = Vec::new();
    for (id, lines) in wanted {
        match backends.resolve(&id) {
            Ok(backend) => {
                versions.insert(id, backend.version());
            }
            Err(d) => {
                let names: Vec<String> = lines.iter().map(|(l, _)| format!("`{l}`")).collect();
                let noun = if names.len() == 1 { "line" } else { "lines" };
                errors.push(Diagnostic {
                    message: format!("{noun} {}: {}", names.join(", "), d.message),
                    ..d.at(lines[0].1)
                });
            }
        }
    }
    if errors.is_empty() {
        Ok(versions)
    } else {
        Err(errors)
    }
}

/// For a locale other than the script's own, puts the translation beside
/// the script (`tour.nl.yaml` for `tour.md`) in place of its English,
/// returning what is out of date in it.
fn translate(
    program: &mut Program,
    script: &Path,
    display: &str,
) -> Result<Vec<String>, Diagnostics> {
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
        Diagnostics(vec![d])
    })?;
    let translation = Translation::from_yaml(&yaml)
        .map_err(|e| Diagnostics(vec![Diagnostic::error(format!("{}: {e}", path.display()))]))?;
    let diags = teleprompt_core::translation::apply(program, &translation);
    if diags.iter().any(Diagnostic::is_error) {
        return Err(Diagnostics(diags));
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
    other_backends: std::collections::BTreeMap<String, String>,
    base_dir: &Path,
) -> Result<CompileOutput, Diagnostics> {
    let cache = VoiceCache::new(project.caches().root);
    let estimator = WpmEstimator::default();
    let version = backend.version();
    let takes = Takes::load(&project.takes_dir())
        .map_err(|e| Diagnostics(vec![Diagnostic::error(e.to_string())]))?;
    let ctx = VoiceContext {
        other_backends,
        backend_id: backend.id(),
        backend_version: &version,
        cache: &cache,
        estimator: &estimator,
        takes: &takes,
    };

    let mut out = compile(
        program,
        crate::scene::plugins(),
        &ctx,
        base_dir,
        env!("CARGO_PKG_VERSION"),
    )?;
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

impl Project {
    /// `script` resolved in its own language, for reading its narration rather
    /// than compiling it.
    pub(crate) fn source_program(&self, script: &Path) -> Result<Program, Vec<String>> {
        self.resolved(script, &self.source_locale())
    }

    /// The language the project's scripts are written in: the locale a command
    /// compiles for when none is given.
    pub fn source_locale(&self) -> String {
        teleprompt_core::config::Config::merged(std::slice::from_ref(&self.config))
            .locales
            .source
    }

    /// `script` resolved for `locale`, untranslated: its configuration as that
    /// locale sees it.
    pub(crate) fn resolved(&self, script: &Path, locale: &str) -> Result<Program, Vec<String>> {
        let project = self;
        let display = script.display().to_string();
        let name = script
            .file_name()
            .map_or_else(|| display.clone(), |s| s.to_string_lossy().to_string());
        let src = std::fs::read_to_string(script)
            .map_err(|e| vec![format!("cannot read {display}: {e}")])?;
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
}

fn render(d: &teleprompt_core::Diagnostics, file: &str) -> Vec<String> {
    d.0.iter().map(|x| x.render(file)).collect()
}
