//! Recording a terminal session with `vhs record`, which writes the tape
//! of what was typed, and reading a tape back as the commands in it.
//!
//! The tape states pauses (`Sleep`) but not how fast each key was typed,
//! so a step's time is what `vhs` would take to play the tape to there:
//! close to when it happened, not exact.

use std::fs::File;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use teleprompt_capture::record::{wait_for, Recorded, Recorder, Recording, Start, Step};
use teleprompt_capture::tool::missing;
use teleprompt_core::{BlockId, Hash};
use teleprompt_scene::{Measured, SceneCompiler, Shot};

use crate::VhsScene;

#[derive(Debug, Default, Clone, Copy)]
pub struct VhsRecorder;

impl Recorder for VhsRecorder {
    fn adapter(&self) -> &'static str {
        "vhs"
    }

    fn unavailable(&self) -> Option<String> {
        missing(&["vhs"])
    }

    fn needs(&self) -> &'static [&'static str] {
        &["vhs"]
    }

    fn in_terminal(&self) -> bool {
        true
    }

    fn extension(&self) -> &'static str {
        "tape"
    }

    fn start(&self, file: &Path, how: &Start) -> Result<Box<dyn Recording>, String> {
        let out =
            File::create(file).map_err(|e| format!("cannot create {}: {e}", file.display()))?;
        let mut command = Command::new("vhs");
        command.arg("record");
        // `vhs record` takes a shell by name: bash, zsh, fish.
        if let Some(shell) = how.shell.first() {
            let name = Path::new(shell)
                .file_name()
                .map_or_else(|| shell.clone(), |n| n.to_string_lossy().into_owned());
            command.args(["--shell", &name]);
        }
        let child = command
            .current_dir(how.cwd)
            .stdout(Stdio::from(out))
            .spawn()
            .map_err(|e| format!("cannot start vhs: {e}"))?;
        Ok(Box::new(Session {
            child,
            file: file.to_path_buf(),
            started: Instant::now(),
        }))
    }

    fn read(&self, text: &str) -> Result<Recorded, String> {
        Ok(read(text))
    }
}

struct Session {
    child: Child,
    file: std::path::PathBuf,
    started: Instant,
}

impl Recording for Session {
    fn started(&self) -> Instant {
        self.started
    }

    /// `vhs record` writes its tape when it is told to stop, as on
    /// SIGTERM, or when the shell exits.
    fn wait(mut self: Box<Self>, stop: &AtomicBool) -> Result<Recorded, String> {
        let pid = self.child.id();
        wait_for(&mut self.child, stop, "TERM", || Some(pid.to_string()))?;
        let text =
            std::fs::read_to_string(&self.file).map_err(|e| format!("vhs wrote no tape: {e}"))?;
        if text.trim().is_empty() {
            return Err("vhs recorded nothing".into());
        }
        Ok(read(&text))
    }
}

/// A tape as steps: one per command, from its first key through `Enter`
/// (or `Ctrl+C`), with the `Sleep` after it. A closing `exit` is left out.
pub fn read(text: &str) -> Recorded {
    let mut recorded = Recorded::default();
    let mut clock: u64 = 0;
    let mut step: Option<Step> = None;
    let mut closing = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let ms = line_ms(trimmed);
        let pause = trimmed.starts_with("Sleep");
        match &mut step {
            // A pause between commands is the end of the one before.
            None if pause => match recorded.steps.last_mut() {
                Some(prev) => prev.text.push_str(&format!("{trimmed}\n")),
                None => recorded.head.push_str(&format!("{trimmed}\n")),
            },
            None => {
                step = Some(Step {
                    start_ms: clock,
                    end_ms: clock,
                    text: String::new(),
                    mark: "# mark\n".into(),
                });
                closing = matches!(trimmed, r#"Type "exit""# | r#"Type "logout""#);
            }
            Some(_) => {}
        }
        clock += ms;
        if let Some(s) = &mut step {
            s.text.push_str(&format!("{trimmed}\n"));
            s.end_ms = clock;
            if ends_command(trimmed) {
                let done = step.take().expect("just matched");
                if !closing {
                    recorded.steps.push(done);
                }
            }
        }
    }
    if let Some(s) = step {
        recorded.steps.push(s);
    }
    recorded
}

fn ends_command(line: &str) -> bool {
    line.starts_with("Enter") || line.starts_with("Ctrl+C") || line.starts_with("Ctrl+D")
}

/// How long `vhs` takes over one line, at its default typing speed.
fn line_ms(line: &str) -> u64 {
    let shot = Shot::numbered(
        &BlockId::new("record"),
        0,
        format!("{line}\n"),
        Hash::of(line.as_bytes()),
    );
    match VhsScene.estimate(&shot) {
        Measured::Exact(ms) | Measured::Estimated(ms) => ms,
        Measured::Unknown => 0,
    }
}
