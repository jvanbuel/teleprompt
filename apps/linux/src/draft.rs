//! A session drafted in the author's terminal, as `teleprompt record
//! --status` reports it.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// The terminal has not started `teleprompt record` yet.
    Starting,
    /// Recording, by the process to stop with SIGTERM.
    Recording(i32),
    Drafting,
    Done(PathBuf),
    Failed(String),
}

/// What the status file at `path` says now.
pub fn progress(path: &Path) -> Progress {
    let Ok(bytes) = std::fs::read(path) else {
        return Progress::Starting;
    };
    let Ok(status) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Progress::Starting;
    };
    match status["state"].as_str() {
        Some("recording") => status["pid"]
            .as_i64()
            .and_then(|p| i32::try_from(p).ok())
            .map_or(Progress::Starting, Progress::Recording),
        Some("drafting") => Progress::Drafting,
        Some("done") => {
            Progress::Done(PathBuf::from(status["script"].as_str().unwrap_or_default()))
        }
        Some("failed") => {
            Progress::Failed(status["error"].as_str().unwrap_or("no reason given").into())
        }
        _ => Progress::Starting,
    }
}
