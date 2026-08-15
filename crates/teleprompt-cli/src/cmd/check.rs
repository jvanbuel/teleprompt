use std::path::Path;

use serde::Serialize;
use teleprompt_compile::{compile, CompileOutput};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::resolve;
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::NullVoice;

use crate::project::Project;

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

    compile(
        &program,
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
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
