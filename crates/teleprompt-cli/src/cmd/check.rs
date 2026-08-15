use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_cache::VoiceCache;
use teleprompt_compile::{compile, CompileOutput, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Item};
use teleprompt_core::{Diagnostic, Diagnostics};
use teleprompt_scene::SceneRegistry;
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
pub fn compile_script(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<CompileOutput, Vec<String>> {
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
    let backend = crate::voice::resolve(&crate::voice::registry(), &program.config.voice.backend)
        .map_err(|e| vec![e])?;

    // Delivery A supports exactly one backend per compile: `VoiceContext`
    // carries a single `backend_id` for the whole program. A segment that
    // writes its own `voice.backend=` and disagrees with the program's
    // resolved backend cannot be honoured — silently ignoring it would let
    // an author believe an override took effect when it did not, and
    // worse, two segments differing only by backend would land on the same
    // cache key and one would be served the other's audio. So it is
    // rejected here rather than either of those.
    let mismatches: Vec<Diagnostic> = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Narration {
                id, config, span, ..
            } if config.voice.backend != program.config.voice.backend => Some(
                Diagnostic::error(format!(
                    "segment `{id}` sets voice.backend=`{}`, but this compile resolved to \
                     backend `{}`; per-segment voice backends are not supported yet",
                    config.voice.backend, program.config.voice.backend
                ))
                .at(*span)
                .with_help(
                    "remove the segment-level voice.backend= override, or change the \
                     project/script default instead",
                ),
            ),
            _ => None,
        })
        .collect();
    if !mismatches.is_empty() {
        return Err(render(&Diagnostics(mismatches), &display));
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

    compile(
        &program,
        &SceneRegistry::with_builtins(),
        &ctx,
        base_dir,
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|d| render(&d, &display))
}

/// Returns warnings on success, rendered errors on failure. `check` is
/// `compile_script` minus the timeline: parse, validate, compile — but the
/// caller only ever looks at whether it succeeded and what it has to say.
pub fn run_check(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<Vec<String>, Vec<String>> {
    compile_script(project, script, locale).map(|out| out.warnings)
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
