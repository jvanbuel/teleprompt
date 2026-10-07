//! Recording an author at work with a scene plugin's own tool, which
//! `teleprompt record` drafts a script from
//! (`docs/design.md#recording-a-session`).
//!
//! The tool does the recording: asciinema, `vhs record`, `playwright
//! codegen`. What a scene plugin adds is reading the tool's file back as timed
//! steps, and knowing how to mark a cut between two of them so the script
//! can `include=` each part.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Why a recording could not be made or read back.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("cannot create {}: {source}", path.display())]
    Create {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The tool's program could not be started.
    #[error("cannot start {tool}: {source}")]
    Start {
        tool: &'static str,
        source: std::io::Error,
    },
    /// The tool ended without leaving the file it records into.
    #[error("{tool} wrote no {what}: {source}")]
    NoOutput {
        tool: &'static str,
        what: &'static str,
        source: std::io::Error,
    },
    /// The tool's file holds no steps.
    #[error("{tool} recorded nothing")]
    Nothing { tool: &'static str },
    #[error("cannot wait for the recording: {0}")]
    Wait(std::io::Error),
    #[error("cannot listen for SIGTERM: {0}")]
    Signal(std::io::Error),
    #[error("`record` needs ffmpeg to record the microphone: {0}")]
    NoFfmpeg(std::io::Error),
    /// ffmpeg ended before any audio was flowing; `log` is what it said.
    #[error(
        "ffmpeg could not record the microphone ({input}): {log}\n  \
         pass ffmpeg's input with --mic, e.g. --mic \"-f alsa -i default\""
    )]
    MicFailed { input: String, log: String },
    #[error("the microphone ({input}) sent nothing for five seconds")]
    MicSilent { input: String },
    /// A recording in the tool's own format that cannot be read as steps,
    /// worded by the scene plugin that knows the format.
    #[error("{0}")]
    Unreadable(String),
}

/// A scene plugin's recording tool.
pub trait Recorder: Send + Sync {
    /// Why it cannot record here, such as its tool not being installed.
    fn unavailable(&self) -> Option<String>;
    /// What it runs, as [`CaptureBackend::needs`](crate::capture::CaptureBackend::needs).
    fn needs(&self) -> &'static [&'static teleprompt_core::tool::Tool];
    /// Whether the author works in the terminal `record` runs in, rather
    /// than a window the tool opens.
    fn in_terminal(&self) -> bool;
    /// The extension its recordings are saved with, without the dot.
    fn extension(&self) -> &'static str;
    /// Starts the tool recording into `file`.
    fn start(&self, file: &Path, how: &Start) -> Result<Box<dyn Recording>, RecordError>;
    /// A recording made with this tool earlier, as steps, for `import`.
    fn read(&self, text: &str) -> Result<Recorded, RecordError>;
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
    fn wait(self: Box<Self>, stop: &AtomicBool) -> Result<Recorded, RecordError>;
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
    /// What cuts the file before it, as the plugin's scene splits: a
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
) -> Result<(), RecordError> {
    let mut asked: Option<Instant> = None;
    loop {
        if child.try_wait().map_err(RecordError::Wait)?.is_some() {
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

/// The rate the microphone is recorded at.
const MIC_RATE: u64 = 48_000;

/// Where a session is recorded, and with what.
pub struct Settings<'a> {
    /// A directory that exists: the microphone's `voice.wav` and the tool's
    /// `session.<extension>` are written here.
    pub into: &'a Path,
    /// What the tool starts from.
    pub start: Start<'a>,
    /// ffmpeg's input arguments for the microphone; the platform's default
    /// input when empty.
    pub mic: &'a [String],
}

/// A session recorded: the tool's recording as steps, the microphone's
/// file beside it, and how far apart their clocks started.
#[derive(Debug)]
pub struct Session {
    pub recorded: Recorded,
    /// The microphone's WAV.
    pub voice: PathBuf,
    /// When the microphone's first sample was taken, minus when the tool's
    /// clock started, in milliseconds: add it to a step's time to find it
    /// in the voice.
    pub offset_ms: i64,
}

/// Records the author at work with `recorder`'s tool, and the microphone
/// beside it with ffmpeg, until the tool finishes or the process is sent
/// SIGTERM, which ends the tool as its own stop would. `recording` is
/// called once both are running.
pub fn record_session(
    recorder: &dyn Recorder,
    settings: &Settings,
    recording: &dyn Fn(),
) -> Result<Session, RecordError> {
    // SIGTERM ends the recording as the tool's own stop would, and the
    // session is still drafted: it is how the app's Stop works.
    let stop = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&stop))
        .map_err(RecordError::Signal)?;

    let voice = settings.into.join("voice.wav");
    let mic = Mic::start(&voice, settings.mic)?;
    recording();
    let file = settings
        .into
        .join(format!("session.{}", recorder.extension()));
    let recorded = recorder
        .start(&file, &settings.start)
        .and_then(|recording| {
            let started = recording.started();
            recording.wait(&stop).map(|rec| (rec, started))
        });
    let mic_started = mic.stop();
    let (recorded, started) = recorded?;
    Ok(Session {
        recorded,
        voice,
        offset_ms: signed_ms(mic_started, started),
    })
}

/// `a - b`, in milliseconds.
fn signed_ms(a: Instant, b: Instant) -> i64 {
    match a.checked_duration_since(b) {
        Some(d) => d.as_millis() as i64,
        None => -(b.duration_since(a).as_millis() as i64),
    }
}

/// The microphone, recorded to a WAV by ffmpeg.
#[must_use = "dropping it stops the recording"]
struct Mic {
    child: Child,
    /// When the first sample was taken, as near as the file's growth says.
    started: Instant,
}

impl Mic {
    fn start(path: &Path, input: &[String]) -> Result<Mic, RecordError> {
        let input: Vec<String> = if input.is_empty() {
            default_mic().iter().map(|s| (*s).to_string()).collect()
        } else {
            input.to_vec()
        };
        // To a file, not a pipe nobody reads while recording: a full pipe
        // would stop ffmpeg mid-session.
        let log = path.with_extension("log");
        let stderr = std::fs::File::create(&log).map_err(|source| RecordError::Create {
            path: log.clone(),
            source,
        })?;
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error"])
            .args(&input)
            .args(["-ac", "1", "-ar", &MIC_RATE.to_string(), "-y"])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .map_err(RecordError::NoFfmpeg)?;
        // Audio is flowing once the file holds more than its header.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let size = std::fs::metadata(path).map_or(0, |m| m.len());
            if size > 4096 {
                let recorded = Duration::from_millis((size - 100) * 1000 / (MIC_RATE * 2));
                return Ok(Mic {
                    started: Instant::now() - recorded,
                    child,
                });
            }
            if let Ok(Some(_)) = child.try_wait() {
                let err = std::fs::read_to_string(&log).unwrap_or_default();
                return Err(RecordError::MicFailed {
                    input: input.join(" "),
                    log: err.trim().to_string(),
                });
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RecordError::MicSilent {
                    input: input.join(" "),
                });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Asks ffmpeg to finish the file, as `q` at its prompt does.
    fn stop(mut self) -> Instant {
        if let Some(mut stdin) = self.child.stdin.take() {
            let _ = stdin.write_all(b"q");
        }
        let _ = self.child.wait();
        self.started
    }
}

/// A recording that ends without [`Mic::stop`], on an error or a panic,
/// still ends ffmpeg's: the microphone is not left recording.
impl Drop for Mic {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// ffmpeg's input for the platform's default microphone.
fn default_mic() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["-f", "avfoundation", "-i", ":0"]
    } else {
        &["-f", "pulse", "-i", "default"]
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// A recording that ends early, on an error or a panic, does not leave
    /// ffmpeg recording the microphone.
    #[test]
    fn a_mic_dropped_unstopped_stops_recording() {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let dir = teleprompt_testkit::test_dir("record-mic-drop");
        let tone: Vec<String> = ["-re", "-f", "lavfi", "-i", "sine=frequency=440"]
            .map(String::from)
            .to_vec();
        let mic = Mic::start(&dir.join("voice.wav"), &tone).unwrap();
        let ffmpeg = std::path::PathBuf::from(format!("/proc/{}", mic.child.id()));
        assert!(ffmpeg.exists());
        drop(mic);
        assert!(!ffmpeg.exists(), "ffmpeg is still running, or unreaped");
    }

    fn io(why: &str) -> std::io::Error {
        std::io::Error::other(why)
    }

    /// The CLI prints these to authors as they are.
    #[test]
    fn every_error_reads_as_it_always_did() {
        let said = |e: RecordError| e.to_string();
        assert_eq!(
            said(RecordError::Create {
                path: "a/voice.log".into(),
                source: io("denied"),
            }),
            "cannot create a/voice.log: denied"
        );
        assert_eq!(
            said(RecordError::Start {
                tool: "vhs",
                source: io("not found"),
            }),
            "cannot start vhs: not found"
        );
        assert_eq!(
            said(RecordError::NoOutput {
                tool: "asciinema",
                what: "recording",
                source: io("gone"),
            }),
            "asciinema wrote no recording: gone"
        );
        assert_eq!(
            said(RecordError::Nothing { tool: "vhs" }),
            "vhs recorded nothing"
        );
        assert_eq!(
            said(RecordError::Wait(io("lost"))),
            "cannot wait for the recording: lost"
        );
        assert_eq!(
            said(RecordError::Signal(io("no"))),
            "cannot listen for SIGTERM: no"
        );
        assert_eq!(
            said(RecordError::NoFfmpeg(io("missing"))),
            "`record` needs ffmpeg to record the microphone: missing"
        );
        assert_eq!(
            said(RecordError::MicFailed {
                input: "-f alsa -i hw".into(),
                log: "no such device".into(),
            }),
            "ffmpeg could not record the microphone (-f alsa -i hw): no such device\n  \
             pass ffmpeg's input with --mic, e.g. --mic \"-f alsa -i default\""
        );
        assert_eq!(
            said(RecordError::MicSilent {
                input: "-i x".into()
            }),
            "the microphone (-i x) sent nothing for five seconds"
        );
        assert_eq!(
            said(RecordError::Unreadable("not a cast".into())),
            "not a cast"
        );
    }
}
