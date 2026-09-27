//! Capturing a script's shots and building its video from the app:
//! `teleprompt capture` and `teleprompt build`, run as a person would,
//! with their `--format json` progress read line by line as they go.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// What the app asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// Record the shots not yet captured, or changed since.
    Capture,
    /// Capture, then render the video.
    Build,
}

/// How far one stage has got: `done` of `of` lines, shots or milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// `voice`, `capture` or `render`.
    pub stage: String,
    pub done: u64,
    pub of: u64,
    /// The line or shot just done; empty for a render.
    pub what: String,
}

impl Progress {
    pub fn fraction(&self) -> f64 {
        if self.of == 0 {
            0.0
        } else {
            self.done.min(self.of) as f64 / self.of as f64
        }
    }

    /// What is being done, as the app says it.
    pub fn label(&self) -> String {
        let count = format!("{} of {}", self.done, self.of);
        match self.stage.as_str() {
            "voice" => format!("Voicing lines · {count}"),
            "capture" => {
                let block = self
                    .what
                    .split_once('#')
                    .map_or(self.what.as_str(), |(b, _)| b);
                format!("Capturing {block} · {count}")
            }
            "render" => format!("Rendering · {:.0}%", self.fraction() * 100.0),
            other => format!("{other} · {count}"),
        }
    }
}

/// A line of the command's stderr, if it is a progress event.
pub fn event(line: &str) -> Option<Progress> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v["event"] != "progress" {
        return None;
    }
    let n = |key: &str| v[key].as_u64();
    let (done, of) = match (n("done"), n("of")) {
        (Some(d), Some(o)) => (d, o),
        _ => (n("done_ms")?, n("of_ms")?),
    };
    let what = ["shot", "line"]
        .iter()
        .find_map(|k| v[*k].as_str())
        .unwrap_or("");
    Some(Progress {
        stage: v["stage"].as_str()?.to_string(),
        done,
        of,
        what: what.to_string(),
    })
}

/// The video a build's report names, or why it made none.
pub fn built(report: &str) -> Result<PathBuf, String> {
    let v: Value =
        serde_json::from_str(report).map_err(|e| format!("cannot read the report: {e}"))?;
    match v["output"].as_str() {
        Some(out) if v["ok"] == true => Ok(PathBuf::from(out)),
        _ => Err(failure(&v)),
    }
}

fn failure(report: &Value) -> String {
    report["errors"]
        .as_array()
        .map(|e| {
            e.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "it failed without saying why".into())
}

/// Runs `job` on `script`, telling `progress` how far each stage has got;
/// then the video, for a build. Blocks until done, so call it off the
/// main thread.
pub fn run(
    binary: &Path,
    script: &Path,
    job: Job,
    mut progress: impl FnMut(Progress),
) -> Result<Option<PathBuf>, String> {
    step(binary, &["capture"], script, &mut progress)?;
    if job == Job::Capture {
        return Ok(None);
    }
    let report = step(binary, &["build"], script, &mut progress)?;
    built(&report).map(Some)
}

/// One command: its progress as it goes, and its report.
fn step(
    binary: &Path,
    args: &[&str],
    script: &Path,
    progress: &mut impl FnMut(Progress),
) -> Result<String, String> {
    let dir = script.parent().unwrap_or(Path::new("."));
    let mut child = Command::new(binary)
        .args(["--format", "json"])
        .args(args)
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
    let stdout = child.stdout.take().expect("piped");
    let report = std::thread::spawn(move || std::io::read_to_string(stdout).unwrap_or_default());
    let mut said = String::new();
    for line in BufReader::new(child.stderr.take().expect("piped")).lines() {
        let line = line.unwrap_or_default();
        match event(&line) {
            Some(p) => progress(p),
            None => {
                said.push_str(&line);
                said.push('\n');
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let report = report.join().unwrap_or_default();
    if status.success() {
        Ok(report)
    } else {
        let why = serde_json::from_str(&report)
            .map(|v: Value| failure(&v))
            .unwrap_or(said);
        Err(why.trim().to_string())
    }
}
