//! Running `teleprompt prompt` for the app, and hearing where it listens.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// The prompter API's version: `docs/design.md#prompter-api-version-1`.
pub const VERSION: &str = "/api/v1";

/// How to run `teleprompt prompt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    /// The `teleprompt` binary, built with `--features listen`.
    pub binary: PathBuf,
    pub script: PathBuf,
    /// An unpacked sherpa-onnx streaming zipformer, to follow a reader by
    /// ear; without one, the script's voice reads it.
    pub model: Option<PathBuf>,
    pub locale: String,
}

impl LaunchRequest {
    /// JSON output, so the app can read where it listens; port 0, so the
    /// OS picks a free one.
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "--format".into(),
            "json".into(),
            "prompt".into(),
            self.script.display().to_string(),
            "--locale".into(),
            self.locale.clone(),
            "--port".into(),
            "0".into(),
        ];
        match &self.model {
            Some(model) => args.extend(["--model".into(), model.display().to_string()]),
            None => args.push("--voice".into()),
        }
        args
    }
}

/// What a launched server reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchEvent {
    /// Serving the API at this origin, such as `http://127.0.0.1:41234`.
    Listening(String),
    /// It stopped, or never started: its errors, or the tail of its stderr.
    Ended(Vec<String>),
}

/// The origin in the listening event, if `line` is one.
pub fn parse_listening(line: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Listening {
        event: String,
        url: String,
        api: String,
    }
    let event: Listening = serde_json::from_str(line).ok()?;
    (event.event == "listening" && event.api == VERSION).then_some(event.url)
}

/// Why a server that exited did: the errors of its JSON error report on
/// stdout, else the last lines of its stderr, else its exit status.
pub fn exit_reasons(stdout: &[u8], stderr: &[u8], status: Option<i32>) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Report {
        errors: Vec<String>,
    }
    if let Ok(report) = serde_json::from_slice::<Report>(stdout) {
        if !report.errors.is_empty() {
            return report.errors;
        }
    }
    let stderr = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    let tail: Vec<String> = lines[lines.len().saturating_sub(5)..]
        .iter()
        .map(|l| l.to_string())
        .collect();
    if tail.is_empty() {
        vec![match status {
            Some(code) => format!("teleprompt exited with status {code}"),
            None => "teleprompt was stopped".to_string(),
        }]
    } else {
        tail
    }
}

/// A running `teleprompt prompt`, stopped when dropped.
pub struct ServerProcess {
    child: Arc<Mutex<Child>>,
}

impl ServerProcess {
    /// Starts the server; `on_event` is called from a background thread
    /// once it listens and once it ends.
    pub fn start(
        request: &LaunchRequest,
        on_event: impl Fn(LaunchEvent) + Send + 'static,
    ) -> std::io::Result<Self> {
        let mut command = Command::new(&request.binary);
        command
            .args(request.args())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        die_with_parent(&mut command);
        let mut child = command.spawn()?;
        let mut stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        let child = Arc::new(Mutex::new(child));
        let errors = std::thread::spawn(move || {
            let mut all = Vec::new();
            let _ = stderr.read_to_end(&mut all);
            all
        });
        let waited = child.clone();
        std::thread::spawn(move || {
            let mut all = Vec::new();
            let mut chunk = [0u8; 4096];
            let mut listening = false;
            while let Ok(n) = stdout.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                all.extend_from_slice(&chunk[..n]);
                if !listening {
                    let text = String::from_utf8_lossy(&all);
                    if let Some(origin) = text.lines().find_map(parse_listening) {
                        listening = true;
                        on_event(LaunchEvent::Listening(origin));
                    }
                }
            }
            let stderr = errors.join().unwrap_or_default();
            let status = waited.lock().ok().and_then(|mut c| c.wait().ok());
            on_event(LaunchEvent::Ended(exit_reasons(
                &all,
                &stderr,
                status.and_then(|s| s.code()),
            )));
        });
        Ok(Self { child })
    }

    pub fn stop(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Has the kernel stop the server when the thread that launched it goes,
/// so an app that is killed or crashes does not leave it running.
#[cfg(target_os = "linux")]
fn die_with_parent(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    let parent = std::process::id();
    // SAFETY: between fork and exec, only async-signal-safe calls: prctl,
    // getppid and _exit.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have gone before the signal was asked for.
            if libc::getppid() as u32 != parent {
                libc::_exit(0);
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn die_with_parent(_: &mut Command) {}
