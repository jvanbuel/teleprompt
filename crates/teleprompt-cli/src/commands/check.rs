use serde::Serialize;
use teleprompt_core::Diagnostics;
use teleprompt_project::project::Script;

/// Compiles the script, writing nothing: its warnings, or rendered
/// errors.
pub fn check(script: &Script) -> Result<Vec<String>, Diagnostics> {
    let compiled = script.compile()?;
    let mut warnings = compiled.output.warnings;
    // What is hard to say aloud, as it is said: translated, for a
    // locale with a translation.
    let display = script.path().display().to_string();
    warnings.extend(
        teleprompt_script::lint::lint(&compiled.program)
            .iter()
            .map(|d| {
                d.render(&display)
                    .trim_start_matches("warning: ")
                    .to_string()
            }),
    );
    Ok(warnings)
}

/// `check`'s `--format json` output, with `ok` as every report has it.
/// Both lists are always present, so a consumer never branches on a
/// missing key.
#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// `check` reports its own failure: its JSON report has room for the errors.
pub fn run(args: crate::cli::ScriptArgs, format: crate::output::Format) -> crate::cli::Run {
    use crate::output::{Format, Outcome};
    match check(&args.open()?) {
        Ok(warnings) => {
            crate::cli::warn(&warnings);
            let report = CheckReport {
                warnings,
                errors: Vec::new(),
            };
            crate::cli::emit(format, &report, "ok\n");
            Ok(Outcome::Ok)
        }
        Err(problems) => {
            let errors = problems.render();
            match format {
                Format::Json => {
                    let report = CheckReport {
                        warnings: Vec::new(),
                        errors: errors.clone(),
                    };
                    crate::cli::emit_ok(format, &report, "", false);
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
    fn check_report_serializes_both_lists() {
        let report = CheckReport {
            warnings: vec!["bare wait".to_string()],
            errors: vec![],
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"warnings": ["bare wait"], "errors": []})
        );
    }
}
