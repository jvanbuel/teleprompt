//! Compiling a script in its project: read, parse, resolve with the
//! project's configuration and the script's translation, and compile with
//! the project's voice backends. Every command that reads a script starts
//! here; `check` is the one that stops.

use std::path::{Path, PathBuf};
use teleprompt_script::program::Program;
use teleprompt_voice::takes::Takes;

use teleprompt_core::{Diagnostic, Diagnostics, LineId, SourceSpan};
use teleprompt_pipeline::compile::{compile, CompileOutput, VoiceContext};
use teleprompt_script::config::PartialConfig;
use teleprompt_script::parse::parse_script;
use teleprompt_script::program::{resolve, Element};
use teleprompt_script::translation::Translation;
use teleprompt_voice::WpmEstimator;

use super::Project;
use teleprompt_voice::backends::{Backends, Cast};

impl Project {
    /// The project's backends, built once from its settings and told which file
    /// those settings came from so a diagnostic about them can point at it.
    pub fn backends(&self) -> Backends {
        let settings = self.config.backends.clone().unwrap_or_default();
        self.registry
            .backends(&settings, &self.config_path().display().to_string())
    }

    /// What `setup` says about this project's voice: its backend's server,
    /// and every `backends:` setting that cannot be used.
    pub async fn voice_status(&self) -> teleprompt_voice::backends::VoiceStatus {
        let backend = self
            .config
            .voice
            .as_ref()
            .and_then(|v| v.backend.clone())
            .unwrap_or_else(|| "null".to_string());
        self.backends().status(&backend).await
    }

    /// `path` in this project, compiled for `locale`.
    pub fn script(&self, path: impl Into<PathBuf>, locale: impl Into<String>) -> Script {
        Script::open(self.clone(), path, locale)
    }
}

/// One script of a project, for one locale, and the voices it is compiled
/// against: what every command that reads a script works on.
#[derive(Clone)]
pub struct Script {
    project: Project,
    path: PathBuf,
    locale: String,
    backends: Backends,
}

impl Script {
    /// With the project's own voices, built from its `backends:` settings
    /// before the script is read, so a script's front-matter `backends:`
    /// cannot reach them (it gets a warning instead).
    pub fn open(project: Project, path: impl Into<PathBuf>, locale: impl Into<String>) -> Self {
        let backends = project.backends();
        Self {
            project,
            path: path.into(),
            locale: locale.into(),
            backends,
        }
    }

    /// With `backends` instead. The seam exists so a test can register a
    /// backend this build does not ship and watch the whole path — key,
    /// synthesis, cache, manifest — follow it (docs/design.md#crates),
    /// which a workspace with one real backend could not otherwise check.
    #[must_use]
    pub fn with_backends(self, backends: Backends) -> Self {
        Self { backends, ..self }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn locale(&self) -> &str {
        &self.locale
    }

    pub fn backends(&self) -> &Backends {
        &self.backends
    }

    /// The same script in its project's source locale: what it says as
    /// written, which is what an edit changes.
    #[must_use]
    pub fn in_source_locale(&self) -> Script {
        Self {
            locale: self.project.source_locale(),
            ..self.clone()
        }
    }

    /// The script as saved: read, parsed, resolved, compiled, its problems
    /// rendered. Writes nothing. The narrator's backend in [`Compiled`] is
    /// the one the cache keys were computed from, so a synthesizing caller
    /// uses it.
    pub fn compile(&self) -> Result<Compiled, Diagnostics> {
        let display = self.path.display().to_string();
        let src = std::fs::read_to_string(&self.path)
            .map_err(|e| Diagnostics::error(format!("cannot read {display}: {e}")))?;
        self.compile_source(&src).map_err(|d| d.about(&display))
    }
}

/// A script compiled: what it compiles to, the backends its lines are
/// spoken by, and the program it was compiled from.
pub struct Compiled {
    pub output: CompileOutput,
    pub cast: Cast,
    pub program: Program,
}

impl Script {
    /// [`Script::compile`] on `src`, the script's text as it may be in an
    /// editor, not yet saved; its problems as diagnostics, not text.
    pub fn compile_source(&self, src: &str) -> Result<Compiled, Diagnostics> {
        let (project, backends, script, locale) = (
            &self.project,
            &self.backends,
            self.path.as_path(),
            self.locale.as_str(),
        );
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
        // settings fail here only for a backend the script is spoken by (see
        // `Backends`).
        let cast = backends
            .cast(&program.config.voice.backend, lines(&program))
            .map_err(Diagnostics)?;

        // `script_dir`, not `script.parent()`: a bare `demo.md` has
        // `Some("")` for a parent, which is not the current directory.
        let base_dir = crate::project::script_dir(script);
        let mut output = compile_with_voice(project, &program, &cast, base_dir)?;

        // The bare message, not `render()`'s output: like every other warning
        // here it is a plain sentence, and callers add their own framing.
        output
            .warnings
            .extend(backend_override_diags.into_iter().map(|d| d.message));
        output.warnings.extend(translation_warnings);
        Ok(Compiled {
            output,
            cast,
            program,
        })
    }
}

/// Each line's backend, id and place in the script, for the cast.
fn lines(program: &Program) -> impl Iterator<Item = (&str, &LineId, SourceSpan)> {
    program.elements.iter().filter_map(|item| match item {
        Element::Narration {
            id, config, span, ..
        } => Some((config.voice.backend.as_str(), id, *span)),
        _ => None,
    })
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
    let diags = teleprompt_script::translation::apply(program, &translation);
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
    cast: &Cast,
    base_dir: &Path,
) -> Result<CompileOutput, Diagnostics> {
    let cache = project.voice_cache();
    let estimator = WpmEstimator::default();
    let takes = project
        .takes()
        .map_err(|e| Diagnostics(vec![Diagnostic::error(e.to_string())]))?;
    let ctx = VoiceContext {
        versions: cast.versions(),
        cache: &cache,
        estimator: &estimator,
        takes: &takes,
    };

    let mut out = compile(
        program,
        project.registry.scenes,
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
    base: &std::collections::BTreeMap<String, serde_json::Value>,
    merged: &std::collections::BTreeMap<String, serde_json::Value>,
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
    before: Option<&serde_json::Value>,
    after: &serde_json::Value,
) -> Vec<String> {
    let (Some(before), Some(after)) = (before.and_then(|v| v.as_object()), after.as_object())
    else {
        return Vec::new();
    };
    after
        .iter()
        .filter(|(k, v)| before.get(*k) != Some(*v))
        .map(|(k, _)| k.clone())
        .collect()
}

impl Project {
    /// `script` resolved in its own language, for reading its narration rather
    /// than compiling it.
    pub(crate) fn source_program(&self, script: &Path) -> Result<Program, Diagnostics> {
        self.resolved(script, &self.source_locale())
    }

    /// The language the project's scripts are written in: the locale a command
    /// compiles for when none is given.
    pub fn source_locale(&self) -> String {
        teleprompt_script::config::Config::merged(std::slice::from_ref(&self.config))
            .locales
            .source
    }

    /// `script` resolved for `locale`, untranslated: its configuration as that
    /// locale sees it.
    pub(crate) fn resolved(&self, script: &Path, locale: &str) -> Result<Program, Diagnostics> {
        let project = self;
        let display = script.display().to_string();
        let name = script
            .file_name()
            .map_or_else(|| display.clone(), |s| s.to_string_lossy().to_string());
        let src = std::fs::read_to_string(script)
            .map_err(|e| Diagnostics::error(format!("cannot read {display}: {e}")))?;
        let parsed = parse_script(&src).map_err(|d| d.about(&display))?;
        resolve(
            &parsed,
            &name,
            locale,
            &project.config,
            &PartialConfig::default(),
        )
        .map_err(|d| d.about(&display))
    }
}
