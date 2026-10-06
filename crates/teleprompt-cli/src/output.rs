use clap::ValueEnum;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Human,
    Json,
}

static JSON_PROGRESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether progress is said as JSON events: set once, from `--format`.
pub fn set_progress_format(format: Format) {
    JSON_PROGRESS.store(format == Format::Json, std::sync::atomic::Ordering::SeqCst);
}

/// Whether progress is said as JSON events, for an app reading them.
pub fn json_progress() -> bool {
    JSON_PROGRESS.load(std::sync::atomic::Ordering::SeqCst)
}

/// One step of a long command, on stderr: the line `human` says, or with
/// `--format json` an event an app reads, `{"event": "progress", "stage":
/// …, …fields}`, one per line.
pub fn progress(stage: &str, human: impl FnOnce() -> String, fields: serde_json::Value) {
    if JSON_PROGRESS.load(std::sync::atomic::Ordering::SeqCst) {
        let mut event = serde_json::json!({ "event": "progress", "stage": stage });
        if let (Some(e), serde_json::Value::Object(f)) = (event.as_object_mut(), fields) {
            e.extend(f);
        }
        eprintln!("{event}");
    } else {
        eprintln!("{}", human());
    }
}

/// One shot of a capture recorded, as [`progress`] says it.
pub fn capture_progress(p: teleprompt_plugin::capture::Progress) {
    progress(
        "capture",
        || format!("  [{}/{}] {} {}", p.done, p.of, p.scene, p.shot),
        serde_json::json!({ "done": p.done, "of": p.of, "scene": p.scene, "shot": p.shot }),
    );
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

/// Why a command failed: the script (exit 2), or anything else (exit 1).
/// The one error every command returns.
#[derive(Debug)]
pub enum Failure {
    /// The script, or what was asked of it: each reason on its own.
    Validation(Vec<String>),
    /// Not the script's fault: a missing tool, a server, a file.
    Runtime(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(reasons) => write!(f, "{}", reasons.join("\n")),
            Self::Runtime(message) => write!(f, "{message}"),
        }
    }
}

impl From<Failure> for Outcome {
    fn from(e: Failure) -> Self {
        match e {
            Failure::Validation(reasons) => Self::ValidationError(reasons),
            Failure::Runtime(message) => Self::RuntimeFailure(message),
        }
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
