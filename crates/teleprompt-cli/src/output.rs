use clap::ValueEnum;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

use teleprompt_core::progress::{Install, Progress, Reporter};
use teleprompt_project::Failure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Human,
    Json,
}

/// Where a command's long steps report: to a person at the terminal as
/// lines on stderr, or with `--format json` as one event a line, which an
/// app reads. The person is asked to install what a step finds missing;
/// an app is not.
pub struct Terminal {
    format: Format,
    /// The last percent of a render shown, so the line is redrawn only
    /// when it changes.
    last_percent: AtomicU64,
}

impl Terminal {
    pub fn new(format: Format) -> Self {
        Self {
            format,
            last_percent: AtomicU64::new(u64::MAX),
        }
    }

    fn human(&self) -> bool {
        self.format == Format::Human
    }

    /// A render's progress, each percent of it, as a line that rewrites
    /// itself with a carriage return, which a log cannot take.
    fn render_line(&self, done_ms: u64, of_ms: u64) {
        use std::io::{IsTerminal, Write};
        if of_ms == 0 || !std::io::stderr().is_terminal() {
            return;
        }
        let percent = (done_ms.min(of_ms) * 100) / of_ms;
        if self.last_percent.swap(percent, Ordering::SeqCst) == percent {
            return;
        }
        let mut err = std::io::stderr();
        let _ = write!(err, "\r  rendering  {percent:>3}%");
        if percent == 100 {
            let _ = writeln!(err);
        }
        let _ = err.flush();
    }
}

impl Reporter for Terminal {
    fn progress(&self, progress: Progress) {
        if self.format == Format::Json {
            let mut event = serde_json::json!({ "event": "progress" });
            if let (Some(e), Ok(serde_json::Value::Object(f))) =
                (event.as_object_mut(), serde_json::to_value(&progress))
            {
                e.extend(f);
            }
            eprintln!("{event}");
            return;
        }
        match progress {
            Progress::Voice { done, of, line } => eprintln!("  [{done}/{of}] {line} done"),
            Progress::Capture {
                done,
                of,
                scene,
                shot,
            } => eprintln!("  [{done}/{of}] {scene} {shot}"),
            Progress::Render { done_ms, of_ms } => self.render_line(done_ms, of_ms),
            Progress::Install { tool, state } => match state {
                Install::Start { command } => eprintln!("installing {tool}: {command}"),
                Install::Done => eprintln!("installed {tool}"),
                Install::Downloading { .. } => {}
            },
        }
    }

    fn note(&self, text: &str) {
        if self.human() {
            eprintln!("{text}");
        }
    }

    fn offer(&self, names: &[&str], why: &str) -> bool {
        self.human() && crate::ask::offer(self, names, why)
    }

    fn attended(&self) -> bool {
        self.human() && crate::ask::interactive()
    }
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

impl From<Failure> for Outcome {
    fn from(e: Failure) -> Self {
        match e {
            Failure::Validation(problems) => Self::ValidationError(problems.render()),
            Failure::Runtime(message) => Self::RuntimeFailure(message),
        }
    }
}

/// Drafting, setting up and serving an editor fail at run time: none of
/// them is the script's fault.
macro_rules! runtime_failures {
    ($($e:ty),*) => {$(
        impl From<$e> for Outcome {
            fn from(e: $e) -> Self {
                Self::RuntimeFailure(e.to_string())
            }
        }
    )*};
}

runtime_failures!(
    teleprompt_draft::DraftError,
    teleprompt_lsp::LspError,
    teleprompt_setup::SetupError
);

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
