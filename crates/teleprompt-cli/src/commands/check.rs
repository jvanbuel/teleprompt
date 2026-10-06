use serde::Serialize;

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
    match args.open()?.check() {
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
