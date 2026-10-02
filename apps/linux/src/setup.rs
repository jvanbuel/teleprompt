//! What `teleprompt setup` can set up, and installing it, as the app asks:
//! `setup --uses` says what each use of teleprompt still needs, and
//! `setup <uses> --run` installs it, saying how it goes as JSON events.
//! The uses, what they need and how each installs are the CLI's.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use serde::Deserialize;

/// One use of teleprompt: render videos, show a browser, follow your voice.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Use {
    pub name: String,
    pub label: String,
    pub listens: bool,
    /// Whether this teleprompt can do it at all.
    pub available: bool,
    pub installed: bool,
    pub download_mb: u32,
    pub tools: Vec<ToolStatus>,
}

/// A tool or model a use needs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ToolStatus {
    pub name: String,
    pub what: String,
    /// `None` where teleprompt cannot tell: a server of your own.
    pub installed: Option<bool>,
    pub license: String,
    pub command: Option<String>,
    pub guide: Option<String>,
    pub download_mb: Option<u32>,
    pub password: bool,
}

impl Use {
    /// What it still needs that `setup` can install.
    pub fn missing(&self) -> Vec<&ToolStatus> {
        self.tools
            .iter()
            .filter(|t| t.installed == Some(false))
            .collect()
    }

    /// What the list says beside it: installed, or what it needs.
    pub fn state(&self) -> String {
        if !self.available {
            return "Needs teleprompt built with speech models".to_string();
        }
        if self.installed {
            return "Installed".to_string();
        }
        let missing = self.missing();
        let (models, programs): (Vec<&&ToolStatus>, Vec<&&ToolStatus>) =
            missing.iter().partition(|t| t.download_mb.is_some());
        let mut parts: Vec<String> = programs.iter().map(|t| t.name.clone()).collect();
        match models.len() {
            0 => {}
            1 => parts.push("a model".to_string()),
            n => parts.push(format!("{n} models")),
        }
        match self.download_mb {
            0 => format!("Needs {}", parts.join(", ")),
            mb => format!("Needs {} · {mb} MB", parts.join(", ")),
        }
    }
}

#[derive(Deserialize)]
struct UsesReport {
    uses: Vec<Use>,
}

/// The uses, as `--uses` printed them.
pub fn parse_uses(json: &str) -> Result<Vec<Use>, String> {
    serde_json::from_str::<UsesReport>(json)
        .map(|r| r.uses)
        .map_err(|e| format!("cannot read what teleprompt can set up: {e}"))
}

/// Asks `binary` what it can be set up to do. Blocks while it looks, so
/// call it off the main thread.
pub fn uses(binary: &Path) -> Result<Vec<Use>, String> {
    let out = Command::new(binary)
        .args(["--format", "json", "setup", "--uses"])
        .output()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
    if !out.status.success() {
        return Err(failure(&out.stdout, &out.stderr));
    }
    parse_uses(&String::from_utf8_lossy(&out.stdout))
}

/// How an install goes, tool by tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Started { tool: String },
    Downloading { tool: String, mb: u32, of: u32 },
    Done { tool: String },
}

/// The step an event line says, if it says one.
pub fn parse_step(line: &str) -> Option<Step> {
    let event: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if event["event"] != "progress" || event["stage"] != "install" {
        return None;
    }
    let tool = event["tool"].as_str()?.to_string();
    let number = |key: &str| event[key].as_u64().and_then(|n| u32::try_from(n).ok());
    match event["state"].as_str()? {
        "start" => Some(Step::Started { tool }),
        "downloading" => Some(Step::Downloading {
            tool,
            mb: number("mb")?,
            of: number("of")?,
        }),
        "done" => Some(Step::Done { tool }),
        _ => None,
    }
}

/// Installs what `uses` need with `binary`, telling `on_step` how it goes.
/// Blocks until it is done, so call it off the main thread. Why it failed,
/// if it did, as teleprompt says it.
pub fn install(binary: &Path, uses: &[String], on_step: impl Fn(Step)) -> Result<(), String> {
    let mut child = Command::new(binary)
        .args(["--format", "json", "setup"])
        .args(uses)
        .arg("--run")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
    let mut stderr = Vec::new();
    if let Some(err) = child.stderr.take() {
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            match parse_step(&line) {
                Some(step) => on_step(step),
                None => stderr.extend_from_slice(format!("{line}\n").as_bytes()),
            }
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("{} stopped: {e}", binary.display()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(failure(&out.stdout, &stderr))
    }
}

/// Why a command failed: the errors its JSON report gives, or else what it
/// printed.
fn failure(stdout: &[u8], stderr: &[u8]) -> String {
    #[derive(Deserialize)]
    struct Failed {
        errors: Vec<String>,
    }
    serde_json::from_slice::<Failed>(stdout)
        .map(|f| f.errors.join("\n"))
        .unwrap_or_else(|_| String::from_utf8_lossy(stderr).trim().to_string())
}

/// The use that sets up what recording with `adapter` needs, if one does.
pub fn use_for_adapter(adapter: &str) -> Option<&'static str> {
    match adapter {
        "vhs" => Some("terminal"),
        "asciinema" => Some("casts"),
        "playwright" => Some("browser"),
        _ => None,
    }
}
