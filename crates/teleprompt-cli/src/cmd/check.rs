use std::path::Path;

use serde::Serialize;

use crate::project::Project;

/// Returns warnings on success, rendered errors on failure.
pub fn run_check(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<Vec<String>, Vec<String>> {
    let mut warnings = project.compile(script, locale)?.0.warnings;
    // What is hard to say aloud, in the language the lines are said in.
    let program = project.resolved(script, locale)?;
    let display = script.display().to_string();
    warnings.extend(teleprompt_core::lint::lint(&program).iter().map(|d| {
        d.render(&display)
            .trim_start_matches("warning: ")
            .to_string()
    }));
    Ok(warnings)
}

/// `check`'s `--format json` output. Both lists are always present, so a
/// consumer never branches on a missing key.
#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub ok: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// `check` reports its own failure: its JSON report has room for the errors.
pub fn run(args: crate::cli::ScriptArgs, format: crate::output::Format) -> crate::cli::Run {
    use crate::output::{Format, Outcome};
    let project = args.project()?;
    match run_check(&project, &args.script, &args.locale(&project)) {
        Ok(warnings) => {
            crate::cli::warn(&warnings);
            let report = CheckReport {
                ok: true,
                warnings,
                errors: Vec::new(),
            };
            crate::cli::emit(format, &report, "ok\n");
            Ok(Outcome::Ok)
        }
        Err(errors) => {
            match format {
                Format::Json => {
                    let report = CheckReport {
                        ok: false,
                        warnings: Vec::new(),
                        errors: errors.clone(),
                    };
                    println!("{}", serde_json::to_string_pretty(&report).unwrap())
                }
                Format::Human => crate::cli::print_errors(&errors),
            }
            Ok(Outcome::ValidationError(errors))
        }
    }
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
