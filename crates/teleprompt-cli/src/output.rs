use clap::ValueEnum;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Human,
    Json,
}

/// The stable, typed shape of a failed command's `--format json` output.
/// Shared by every command whose success payload isn't itself the natural
/// place to carry an `errors` list (`plan` prints a `Timeline`, `diff`
/// prints a `TimelineDiff` — neither has room for `errors`), so their
/// failure paths converge on this one struct instead of each inventing its
/// own. `check` has its own `CheckReport` because it needs `warnings`
/// alongside `errors` even on success; this one is failure-only, hence no
/// `ok: true` case.
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
    VoiceDowngrade,
}

pub fn exit_code_for(outcome: &Outcome) -> i32 {
    match outcome {
        Outcome::Ok => 0,
        Outcome::RuntimeFailure(_) => 1,
        Outcome::ValidationError(_) => 2,
        Outcome::Drift => 3,
        Outcome::VoiceDowngrade => 4,
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
        assert_eq!(exit_code_for(&Outcome::VoiceDowngrade), 4);
    }

    #[test]
    fn error_report_serializes_ok_false_and_the_error_list() {
        let report = ErrorReport::new(vec!["bad".to_string()]);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json, serde_json::json!({"ok": false, "errors": ["bad"]}));
    }
}
