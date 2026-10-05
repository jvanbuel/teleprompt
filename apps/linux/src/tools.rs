//! The tools `teleprompt record` can record a session with, as
//! `teleprompt record --tools` lists them, for session mode to offer.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Tool {
    /// The plugin it records for: `--with`'s value.
    pub plugin: String,
    /// Whether the author works in the terminal, or a window of its own.
    pub in_terminal: bool,
    /// Why it cannot record here, if it cannot.
    pub unavailable: Option<String>,
}

impl Tool {
    /// How the choice reads: what is recorded, then the tool.
    pub fn label(&self) -> String {
        let what = if self.in_terminal {
            "Terminal"
        } else {
            "Browser"
        };
        format!("{what} · {}", self.plugin)
    }
}

/// The list `--tools` printed, as JSON.
pub fn parse(json: &str) -> Result<Vec<Tool>, String> {
    serde_json::from_str(json).map_err(|e| format!("cannot read the tools: {e}"))
}

/// Asks `binary` which tools it records with. Blocks while the tools are
/// looked for, so call it off the main thread.
pub fn list(binary: &Path) -> Result<Vec<Tool>, String> {
    let out = Command::new(binary)
        .args(["--format", "json", "record", "--tools"])
        .output()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// The tool to record with: `chosen` if it can, else the first that can.
pub fn pick<'a>(tools: &'a [Tool], chosen: Option<&str>) -> Option<&'a Tool> {
    let ready = |t: &&Tool| t.unavailable.is_none();
    tools
        .iter()
        .filter(ready)
        .find(|t| Some(t.plugin.as_str()) == chosen)
        .or_else(|| tools.iter().find(ready))
}

/// Whether `teleprompt setup` says what drafting needs is installed (the
/// speech model), from its `--uses` report. `true` when that cannot be
/// read: `record` then says itself what it is missing.
pub fn drafts_ready(report: &str) -> bool {
    let Ok(report) = serde_json::from_str::<serde_json::Value>(report) else {
        return true;
    };
    report["uses"]
        .as_array()
        .and_then(|uses| uses.iter().find(|u| u["name"] == "drafts"))
        .and_then(|u| u["installed"].as_bool())
        .unwrap_or(true)
}

/// Asks `binary` whether drafting is set up, as [`drafts_ready`] reads it.
/// Blocks, so call it off the main thread.
pub fn ask_drafts_ready(binary: &Path) -> bool {
    Command::new(binary)
        .args(["--format", "json", "setup", "--uses"])
        .output()
        .map_or(true, |out| {
            drafts_ready(&String::from_utf8_lossy(&out.stdout))
        })
}
