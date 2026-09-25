use clap::ValueEnum;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Human,
    Json,
}

/// A failed command's `--format json` output, for every command whose
/// success payload has no room for `errors`. Always `ok: false`.
#[derive(Debug, Serialize)]
pub struct ErrorReport {
    pub ok: bool,
    pub errors: Vec<String>,
}

impl ErrorReport {
    pub fn new(errors: Vec<String>) -> Self {
        Self { ok: false, errors }
    }
}

/// Every way a command can end. Centralised so no subcommand invents a code.
#[derive(Debug)]
pub enum Outcome {
    Ok,
    RuntimeFailure(String),
    ValidationError(Vec<String>),
    Drift,
}

pub fn exit_code_for(outcome: &Outcome) -> i32 {
    match outcome {
        Outcome::Ok => 0,
        Outcome::RuntimeFailure(_) => 1,
        Outcome::ValidationError(_) => 2,
        Outcome::Drift => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_for_matches_the_spec() {
        assert_eq!(exit_code_for(&Outcome::Ok), 0);
        assert_eq!(exit_code_for(&Outcome::RuntimeFailure("boom".into())), 1);
        assert_eq!(
            exit_code_for(&Outcome::ValidationError(vec!["bad".into()])),
            2
        );
        assert_eq!(exit_code_for(&Outcome::Drift), 3);
    }

    #[test]
    fn error_report_serializes_ok_false_and_the_error_list() {
        let report = ErrorReport::new(vec!["bad".to_string()]);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json, serde_json::json!({"ok": false, "errors": ["bad"]}));
    }
}
