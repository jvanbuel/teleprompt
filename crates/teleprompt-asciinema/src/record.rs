//! Recording a terminal session with `asciinema rec`, keystrokes and all,
//! and reading a cast back as the commands typed in it.
//!
//! A draft includes the cast itself, split by marker events between the
//! commands: what the video shows is what was recorded, not a re-run.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde_json::Value;
use teleprompt_capture::record::{wait_for, Recorded, Recorder, Recording, Start, Step};
use teleprompt_capture::tool::missing;

#[derive(Debug, Default, Clone, Copy)]
pub struct AsciinemaRecorder;

impl Recorder for AsciinemaRecorder {
    fn adapter(&self) -> &'static str {
        "asciinema"
    }

    fn unavailable(&self) -> Option<String> {
        missing(&["asciinema"])
    }

    fn needs(&self) -> &'static [&'static str] {
        &["asciinema"]
    }

    fn in_terminal(&self) -> bool {
        true
    }

    fn extension(&self) -> &'static str {
        "cast"
    }

    fn start(&self, file: &Path, how: &Start) -> Result<Box<dyn Recording>, String> {
        // The shell writes its pid before it becomes the author's shell, so
        // a stop can hang it up: asciinema then finishes the cast as if it
        // had exited. asciinema 2 ignores SIGTERM.
        let pid_file = file.with_extension("pid");
        let shell = if how.shell.is_empty() {
            vec![std::env::var("SHELL").unwrap_or_else(|_| "sh".into())]
        } else {
            how.shell.to_vec()
        };
        let command = format!(
            "echo $$ > {}; exec {}",
            quote(&pid_file.to_string_lossy()),
            shell.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
        );
        let input = if major_version() >= 3 {
            "--capture-input"
        } else {
            "--stdin"
        };
        let _ = std::fs::remove_file(file);
        let child = Command::new("asciinema")
            .args(["rec", input, "--quiet", "--overwrite", "--command"])
            .arg(&command)
            .arg(file)
            .current_dir(how.cwd)
            .spawn()
            .map_err(|e| format!("cannot start asciinema: {e}"))?;
        // Its clock starts as it writes the header.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !file.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(Box::new(Session {
            child,
            file: file.to_path_buf(),
            pid_file,
            started: Instant::now(),
        }))
    }

    fn read(&self, text: &str) -> Result<Recorded, String> {
        read(text)
    }
}

struct Session {
    child: Child,
    file: PathBuf,
    pid_file: PathBuf,
    started: Instant,
}

impl Recording for Session {
    fn started(&self) -> Instant {
        self.started
    }

    fn wait(mut self: Box<Self>, stop: &AtomicBool) -> Result<Recorded, String> {
        let pid_file = self.pid_file.clone();
        wait_for(&mut self.child, stop, "HUP", || {
            let pid = std::fs::read_to_string(&pid_file).ok()?;
            Some(pid.trim().to_string()).filter(|p| !p.is_empty())
        })?;
        let _ = std::fs::remove_file(&self.pid_file);
        let text = std::fs::read_to_string(&self.file)
            .map_err(|e| format!("asciinema wrote no recording: {e}"))?;
        read(&text)
    }
}

/// asciinema's major version: 3 takes `--capture-input` where 2 took
/// `--stdin`.
fn major_version() -> u32 {
    let out = Command::new("asciinema").arg("--version").output();
    out.ok()
        .and_then(|o| {
            let text = String::from_utf8_lossy(&o.stdout).into_owned();
            text.split_whitespace()
                .find_map(|w| w.split('.').next()?.parse().ok())
        })
        .unwrap_or(2)
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// One event, at an absolute time in seconds.
struct Event {
    time: f64,
    code: String,
    data: String,
}

/// A cast, v2 or v3, as steps: one per command typed, from its first key
/// to the next command's, with everything the terminal wrote meanwhile.
/// It is rewritten as v2, whose times are absolute, so a marker can go
/// anywhere. The command that closed the shell is left out.
pub fn read(text: &str) -> Result<Recorded, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Value = lines
        .next()
        .and_then(|l| serde_json::from_str(l).ok())
        .ok_or("the cast has no header line")?;
    let relative = match header["version"].as_u64() {
        Some(2) => false,
        Some(3) => true,
        _ => return Err("only asciicast v2 and v3 are read".into()),
    };
    let mut events = Vec::new();
    let mut clock = 0.0;
    for line in lines {
        let v: Value = serde_json::from_str(line).map_err(|_| format!("not an event: {line}"))?;
        let (Some(t), Some(code)) = (v[0].as_f64(), v[1].as_str()) else {
            return Err(format!("not an event: {line}"));
        };
        clock = if relative { clock + t } else { t };
        events.push(Event {
            time: clock,
            code: code.to_string(),
            data: v[2].as_str().unwrap_or("").to_string(),
        });
    }
    if !events.iter().any(|e| e.code == "i") {
        return Err(
            "the cast has no keystrokes: record it with `asciinema rec --stdin` \
             (asciinema 2) or `--capture-input` (asciinema 3)"
                .into(),
        );
    }

    let (firsts, end) = commands(&events);
    let size = if relative {
        (
            header["term"]["cols"].as_u64(),
            header["term"]["rows"].as_u64(),
        )
    } else {
        (header["width"].as_u64(), header["height"].as_u64())
    };
    // asciinema's own size when a cast does not say.
    let (cols, rows) = (size.0.unwrap_or(80), size.1.unwrap_or(24));
    let mut v2 = serde_json::json!({ "version": 2, "width": cols, "height": rows });
    match header.get("idle_time_limit") {
        Some(idle) => v2["idle_time_limit"] = idle.clone(),
        // Its length is stated, so a last command typed but not yet echoed
        // is still a part: the scene times a cast by what it shows. (A
        // stated length is not capped by an idle limit, so not with one.)
        None => {
            let length = events[..end].last().map_or(0.0, |e| e.time);
            v2["duration"] = serde_json::json!((length * 1_000_000.0).round() / 1_000_000.0);
        }
    }
    let line = |e: &Event| {
        let t = (e.time * 1_000_000.0).round() / 1_000_000.0;
        format!("{}\n", serde_json::json!([t, e.code, e.data]))
    };
    let first = firsts.first().copied().unwrap_or(end);
    let mut recorded = Recorded {
        head: format!("{v2}\n") + &events[..first].iter().map(line).collect::<String>(),
        steps: Vec::new(),
    };
    for (i, &from) in firsts.iter().enumerate() {
        let window = &events[from..firsts.get(i + 1).copied().unwrap_or(end)];
        let at = events[from].time;
        recorded.steps.push(Step {
            start_ms: ms(at),
            end_ms: ms(window.last().map_or(at, |e| e.time)),
            text: window.iter().map(line).collect(),
            mark: format!("{}\n", serde_json::json!([at, "m", ""])),
        });
    }
    Ok(recorded)
}

fn ms(seconds: f64) -> u64 {
    (seconds * 1000.0).round() as u64
}

/// Where each command's first key is, and where the recording worth
/// keeping ends. A command ends at Enter, Ctrl+C or Ctrl+D; a last one
/// that closed the shell (`exit`, Ctrl+D) is not kept, and the recording
/// ends where it began.
fn commands(events: &[Event]) -> (Vec<usize>, usize) {
    let mut firsts = Vec::new();
    let mut closing = None;
    let mut open: Option<String> = None;
    for (i, e) in events.iter().enumerate().filter(|(_, e)| e.code == "i") {
        if open.is_none() {
            firsts.push(i);
        }
        let typed = open.get_or_insert_with(String::new);
        for c in e.data.chars() {
            if matches!(c, '\r' | '\n' | '\u{3}' | '\u{4}') {
                let closes =
                    matches!(typed.trim(), "exit" | "logout") || (c == '\u{4}' && typed.is_empty());
                closing = closes.then_some(i);
                open = None;
                break;
            }
            if !c.is_control() {
                typed.push(c);
            }
        }
    }
    match closing.filter(|_| open.is_none()) {
        Some(_) => {
            let end = firsts.pop().unwrap_or(events.len());
            (firsts, end)
        }
        None => (firsts, events.len()),
    }
}
