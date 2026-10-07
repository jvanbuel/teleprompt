//! Recording a browser session with `playwright codegen`, which writes
//! the script of what was clicked and typed as it happens.
//!
//! The script states no timing, so the recorder watches the file and
//! times each statement by when it appeared. A script recorded some other
//! way cannot be imported for the same reason.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use teleprompt_scene::record::{wait_for, Recorded, Recorder, Recording, Start, Step};

#[derive(Debug, Default, Clone, Copy)]
pub struct PlaywrightRecorder;

impl Recorder for PlaywrightRecorder {
    fn unavailable(&self) -> Option<String> {
        let ok = playwright()
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        (!ok).then(|| "Playwright is not installed (npm install playwright)".to_string())
    }

    fn needs(&self) -> &'static [&'static teleprompt_scene::core::tool::Tool] {
        static NEEDS: &[&teleprompt_scene::core::tool::Tool] = &[
            &teleprompt_scene::core::tool::NODE,
            &crate::playwright::tools::PLAYWRIGHT,
        ];
        NEEDS
    }

    fn in_terminal(&self) -> bool {
        false
    }

    fn extension(&self) -> &'static str {
        "js"
    }

    fn start(&self, file: &Path, how: &Start) -> Result<Box<dyn Recording>, String> {
        let _ = std::fs::remove_file(file);
        let mut command = playwright();
        command
            .args(["codegen", "--target", "javascript", "--output"])
            .arg(file)
            .args(how.url)
            .current_dir(how.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Its own group, so a stop reaches the browser behind npx.
            .process_group(0);
        let child = command
            .spawn()
            .map_err(|e| format!("cannot start playwright codegen: {e}"))?;
        let started = Instant::now();
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let done = Arc::new(AtomicBool::new(false));
        let watcher = {
            let (file, seen, done) = (file.to_path_buf(), Arc::clone(&seen), Arc::clone(&done));
            std::thread::spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    if let Ok(text) = std::fs::read_to_string(&file) {
                        lock(&seen).update(&statements(&text), started.elapsed());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            })
        };
        Ok(Box::new(Session {
            child,
            file: file.to_path_buf(),
            started,
            seen,
            done,
            watcher,
        }))
    }

    fn read(&self, _text: &str) -> Result<Recorded, String> {
        Err(
            "a Playwright script does not say when each step happened, so it cannot be \
             drafted against a voice; record it with `teleprompt record --with playwright`"
                .into(),
        )
    }
}

/// `playwright`, the project's own or the one installed globally.
fn playwright() -> Command {
    let mut c = Command::new("npx");
    c.args(["--no-install", "playwright"]);
    c
}

struct Session {
    child: Child,
    file: PathBuf,
    started: Instant,
    seen: Arc<Mutex<Seen>>,
    done: Arc<AtomicBool>,
    watcher: JoinHandle<()>,
}

impl Recording for Session {
    fn started(&self) -> Instant {
        self.started
    }

    fn wait(mut self: Box<Self>, stop: &AtomicBool) -> Result<Recorded, String> {
        let group = format!("-{}", self.child.id());
        wait_for(&mut self.child, stop, "TERM", || Some(group.clone()))?;
        self.done.store(true, Ordering::SeqCst);
        let _ = self.watcher.join();
        let text = std::fs::read_to_string(&self.file)
            .map_err(|e| format!("playwright codegen wrote no script: {e}"))?;
        let seen = lock(&self.seen);
        Ok(timed(&statements(&text), &seen.times))
    }
}

/// When each statement of the script was first seen.
#[derive(Debug, Default)]
pub struct Seen {
    pub times: Vec<u64>,
}

impl Seen {
    /// codegen rewrites the file on every action, sometimes replacing its
    /// last statement (a `fill` grows as the author types): a statement
    /// keeps the time it first appeared at its place.
    pub fn update(&mut self, statements: &[String], now: Duration) {
        self.times.truncate(statements.len());
        let now = now.as_millis() as u64;
        while self.times.len() < statements.len() {
            self.times.push(now);
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The script's statements, as a scene block runs them: what codegen
/// writes between opening the page and closing the browser.
pub fn statements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("const page = await context.newPage()") {
            inside = true;
        } else if line.starts_with("// ----") || line.starts_with("await context.close()") {
            inside = false;
        } else if inside && !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

/// Statements with their times as steps; one without a time takes the
/// last one's.
pub fn timed(statements: &[String], times: &[u64]) -> Recorded {
    let mut last = 0;
    Recorded {
        head: String::new(),
        steps: statements
            .iter()
            .enumerate()
            .map(|(i, s)| {
                last = times.get(i).copied().unwrap_or(last);
                Step {
                    start_ms: last,
                    end_ms: last,
                    text: format!("{s}\n"),
                    mark: "// mark\n".into(),
                }
            })
            .collect(),
    }
}
