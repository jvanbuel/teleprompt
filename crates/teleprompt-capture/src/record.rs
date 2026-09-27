//! Recording an author at work with an adapter's own tool, which
//! `teleprompt record` drafts a script from
//! (`docs/design.md#recording-a-session`).
//!
//! The tool does the recording: asciinema, `vhs record`, `playwright
//! codegen`. What an adapter adds is reading the tool's file back as timed
//! steps, and knowing how to mark a cut between two of them so the script
//! can `include=` each part.

use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// An adapter's recording tool.
pub trait Recorder: Send + Sync {
    /// The adapter it records for, and the scene its drafts run in.
    fn adapter(&self) -> &'static str;
    /// Why it cannot record here, such as its tool not being installed.
    fn unavailable(&self) -> Option<String>;
    /// Whether the author works in the terminal `record` runs in, rather
    /// than a window the tool opens.
    fn in_terminal(&self) -> bool;
    /// The extension its recordings are saved with, without the dot.
    fn extension(&self) -> &'static str;
    /// Starts the tool recording into `file`.
    fn start(&self, file: &Path, how: &Start) -> Result<Box<dyn Recording>, String>;
    /// A recording made with this tool earlier, as steps, for `import`.
    fn read(&self, text: &str) -> Result<Recorded, String>;
}

/// What a recording starts from.
#[derive(Debug, Clone)]
pub struct Start<'a> {
    /// Where the author starts.
    pub cwd: &'a Path,
    /// A terminal tool's shell and its arguments; the tool's own default
    /// when empty.
    pub shell: &'a [String],
    /// A browser tool's first page.
    pub url: Option<&'a str>,
}

/// A recording under way.
pub trait Recording: Send {
    /// When its clock started: step times count from here.
    fn started(&self) -> Instant;
    /// Until the author finishes, or `stop` is set and the tool is stopped
    /// as its own stop key would; then what was recorded, as steps.
    fn wait(self: Box<Self>, stop: &AtomicBool) -> Result<Recorded, String>;
}

/// A recording as steps: what the author did, when, and the file's text
/// for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recorded {
    /// The file before its first step: a cast's header, a first `Sleep`.
    pub head: String,
    pub steps: Vec<Step>,
}

/// One thing the author did: a command typed, a click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// When it began and ended, in milliseconds on the recording's clock.
    pub start_ms: u64,
    pub end_ms: u64,
    /// Its part of the file.
    pub text: String,
    /// What cuts the file before it, as the adapter's scene splits: a
    /// `# mark` line, a cast's marker event.
    pub mark: String,
}

impl Recorded {
    /// The file with a mark before each step in `cuts`, so its part `n + 1`
    /// is what follows the `n`th cut.
    pub fn marked(&self, cuts: &[usize]) -> String {
        let mut out = self.head.clone();
        for (i, step) in self.steps.iter().enumerate() {
            if cuts.contains(&i) {
                out.push_str(&step.mark);
            }
            out.push_str(&step.text);
        }
        out
    }
}

/// Waits for `child` to exit; once `stop` is set, `signal` is sent to
/// `target()` (a pid, or `-pgid` for a process group: the tool, or the
/// shell it records) to end it as its own stop would, and it is killed if
/// that has not worked within five seconds.
pub fn wait_for(
    child: &mut Child,
    stop: &AtomicBool,
    signal: &str,
    target: impl Fn() -> Option<String>,
) -> Result<(), String> {
    let mut asked: Option<Instant> = None;
    loop {
        if child
            .try_wait()
            .map_err(|e| format!("cannot wait for the recording: {e}"))?
            .is_some()
        {
            return Ok(());
        }
        match asked {
            None if stop.load(Ordering::SeqCst) => {
                if let Some(target) = target() {
                    let _ = Command::new("kill")
                        .args([&format!("-{signal}"), "--", &target])
                        .status();
                }
                asked = Some(Instant::now());
            }
            Some(at) if at.elapsed() > Duration::from_secs(5) => {
                let _ = child.kill();
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
