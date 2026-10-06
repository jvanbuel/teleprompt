//! `teleprompt record <script> --with <plugin>`: the plugin's own tool
//! records the author at work (asciinema, `vhs record`, `playwright
//! codegen`), and ffmpeg records the microphone beside it. When the tool
//! finishes, the two are drafted into `<script>` (`crate::cmd::draft::import`).

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use teleprompt_plugin::record::Start;

use crate::scene::Recording;

use crate::cmd::draft::import::{draft_session, refuse_to_replace, ImportReport, Session, Words};
use crate::project::Project;

pub struct Record<'a> {
    pub script: &'a Path,
    /// The plugin whose tool records; asciinema when `None`.
    pub with: Option<&'a str>,
    pub model: &'a Path,
    /// A punctuation model's directory, as for `import`.
    pub punctuation: Option<&'a Path>,
    /// ffmpeg's input arguments for the microphone; the platform's default
    /// input when empty.
    pub mic: Vec<String>,
    /// A terminal tool's shell; `$SHELL` when empty.
    pub shell: Vec<String>,
    /// A browser tool's first page.
    pub url: Option<&'a str>,
    pub force: bool,
    /// Where to say how the recording is going, as JSON, for an app that
    /// cannot wait on it.
    pub status: Option<&'a Path>,
    /// Print nothing into the terminal but errors.
    pub quiet: bool,
}

/// The rate the microphone is recorded at.
const MIC_RATE: u64 = 48_000;

pub fn run_record(r: &Record) -> Result<ImportReport, String> {
    let say = |state: serde_json::Value| {
        if let Some(path) = r.status {
            let partial = path.with_extension("partial");
            let _ = std::fs::write(&partial, state.to_string())
                .and_then(|()| std::fs::rename(&partial, path));
        }
    };
    let result = recorder(r.with).and_then(|recorder| {
        check(r, &recorder)?;
        record(r, &recorder, &say)
    });
    say(match &result {
        Ok(report) => serde_json::json!({
            "state": "done",
            "script": report.created,
            "lines": report.lines,
            "blocks": report.blocks,
        }),
        Err(e) => serde_json::json!({ "state": "failed", "error": e }),
    });
    result
}

/// The recorder `with` names, asciinema's by default.
fn recorder(with: Option<&str>) -> Result<Recording, String> {
    let all = crate::scene::recorders();
    let names: Vec<&str> = all.iter().map(|r| r.plugin).collect();
    let names = names.join(", ");
    let wanted = with.unwrap_or("asciinema");
    all.into_iter()
        .find(|r| r.plugin == wanted)
        .ok_or_else(|| format!("`{wanted}` cannot record a session; these can: {names}"))
}

/// Everything that could stop the draft, checked before anything is
/// recorded rather than after the author has talked for ten minutes.
fn check(r: &Record, recorder: &Recording) -> Result<(), String> {
    refuse_to_replace(r.script, r.force)?;
    if !cfg!(feature = "listen") {
        return Err(
            "this teleprompt was built without a speech recognizer: rebuild it \
                    with `--features listen`"
                .to_string(),
        );
    }
    if !r.model.is_dir() {
        return Err(format!("no speech model at {}", r.model.display()));
    }
    if let Some(dir) = r.punctuation.filter(|d| !d.is_dir()) {
        return Err(format!("no punctuation model at {}", dir.display()));
    }
    if let Some(why) = recorder.unavailable() {
        return Err(format!("cannot record with {}: {why}", recorder.plugin));
    }
    Ok(())
}

fn record(
    r: &Record,
    recorder: &Recording,
    say: &dyn Fn(serde_json::Value),
) -> Result<ImportReport, String> {
    let project = Project::for_script(r.script).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let stem = r.script.file_stem().unwrap_or_default().to_string_lossy();
    let dir = project
        .root
        .join(".teleprompt/traces")
        .join(format!("{stem}-{stamp}"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    // SIGTERM ends the recording as the tool's own stop would, and the
    // session is still drafted: it is how the app's Stop works.
    let stop = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&stop))
        .map_err(|e| format!("cannot listen for SIGTERM: {e}"))?;

    let voice = dir.join("voice.wav");
    let mic = Mic::start(&voice, &r.mic)?;
    say(serde_json::json!({
        "state": "recording",
        "pid": std::process::id(),
        "trace": dir,
    }));
    if !r.quiet {
        let how = if recorder.in_terminal() {
            "talk as you work, then exit the shell to finish"
        } else {
            "talk as you work in its window, then close it to finish"
        };
        eprintln!(
            "recording with {} and the microphone: {how}\r",
            recorder.plugin
        );
    }
    let file = dir.join(format!("session.{}", recorder.extension()));
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let recorded = recorder
        .start(
            &file,
            &Start {
                cwd: &cwd,
                shell: &r.shell,
                url: r.url,
            },
        )
        .and_then(|recording| {
            let started = recording.started();
            recording.wait(&stop).map(|rec| (rec, started))
        });
    let mic_started = mic.stop();
    let (recorded, started) = recorded?;

    say(serde_json::json!({ "state": "drafting" }));
    if !r.quiet {
        eprintln!("transcribing {}", voice.display());
    }
    draft_session(&Session {
        script: r.script,
        recorder,
        recorded: &recorded,
        voice: &voice,
        words: &Words::Model(r.model),
        offset_ms: signed_ms(mic_started, started),
        punctuation: r.punctuation,
        force: r.force,
    })
}

/// `a - b`, in milliseconds.
fn signed_ms(a: Instant, b: Instant) -> i64 {
    match a.checked_duration_since(b) {
        Some(d) => d.as_millis() as i64,
        None => -(b.duration_since(a).as_millis() as i64),
    }
}

/// A recorder, as `record --tools` lists it for an app to offer.
#[derive(Debug, Serialize)]
pub struct Tool {
    pub plugin: &'static str,
    /// Whether the author works in the terminal, or a window of its own.
    pub in_terminal: bool,
    /// Why it cannot record here, if it cannot.
    pub unavailable: Option<String>,
}

/// Every recorder this build has, the default first.
pub fn tools() -> Vec<Tool> {
    crate::scene::recorders()
        .iter()
        .map(|r| Tool {
            plugin: r.plugin,
            in_terminal: r.in_terminal(),
            unavailable: r.unavailable(),
        })
        .collect()
}

/// The microphone, recorded to a WAV by ffmpeg.
struct Mic {
    child: Child,
    /// When the first sample was taken, as near as the file's growth says.
    started: Instant,
}

impl Mic {
    fn start(path: &Path, input: &[String]) -> Result<Mic, String> {
        let input: Vec<String> = if input.is_empty() {
            default_mic().iter().map(|s| (*s).to_string()).collect()
        } else {
            input.to_vec()
        };
        // To a file, not a pipe nobody reads while recording: a full pipe
        // would stop ffmpeg mid-session.
        let log = path.with_extension("log");
        let stderr = std::fs::File::create(&log)
            .map_err(|e| format!("cannot create {}: {e}", log.display()))?;
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error"])
            .args(&input)
            .args(["-ac", "1", "-ar", &MIC_RATE.to_string(), "-y"])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .map_err(|e| format!("`record` needs ffmpeg to record the microphone: {e}"))?;
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
                return Err(format!(
                    "ffmpeg could not record the microphone ({}): {}\n  \
                     pass ffmpeg's input with --mic, e.g. --mic \"-f alsa -i default\"",
                    input.join(" "),
                    err.trim()
                ));
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "the microphone ({}) sent nothing for five seconds",
                    input.join(" ")
                ));
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

/// What `record` records, and where it writes.
#[derive(clap::Args)]
pub struct Args {
    /// The script to write, e.g. scripts/tour.md
    #[arg(required_unless_present = "tools")]
    pub script: Option<std::path::PathBuf>,
    /// The tool to record with, by its scene plugin: asciinema (the default),
    /// vhs or playwright
    #[arg(long)]
    pub with: Option<String>,
    /// List the tools this build can record with, and whether each is
    /// installed
    #[arg(long)]
    pub tools: bool,
    /// Directory of an unpacked sherpa-onnx streaming zipformer model;
    /// defaults to the one `teleprompt setup speech-model` installed
    #[arg(long)]
    pub model: Option<std::path::PathBuf>,
    /// Directory of an unpacked sherpa-onnx punctuation model, to give the
    /// narration capitals and punctuation
    #[arg(long)]
    pub punctuation: Option<std::path::PathBuf>,
    /// ffmpeg's input for the microphone, e.g. "-f alsa -i default";
    /// defaults to the system's default input
    #[arg(long, allow_hyphen_values = true, env = "TELEPROMPT_RECORD_MIC")]
    pub mic: Option<String>,
    /// The page a browser tool starts on
    #[arg(long)]
    pub url: Option<String>,
    /// Replace the script, its recording and its lines' takes, if they
    /// exist
    #[arg(long)]
    pub force: bool,
    /// Write how the recording is going to this file, as JSON: recording
    /// (with its pid), drafting, done or failed
    #[arg(long)]
    pub status: Option<std::path::PathBuf>,
    /// Leave the terminal to the shell: say nothing but errors, for an app
    /// that shows the recording's progress itself
    #[arg(long, short)]
    pub quiet: bool,
    /// A terminal tool's shell instead of $SHELL, with its arguments
    #[arg(last = true)]
    pub shell: Vec<String>,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    use crate::cli::{emit, emit_data, runtime_failure};
    use crate::cmd::setup;
    use crate::output::Outcome;
    if args.tools {
        let tools = tools();
        let human: String = tools
            .iter()
            .map(|t| {
                format!(
                    "{:<12}{}\n",
                    t.plugin,
                    t.unavailable.as_deref().unwrap_or("ready")
                )
            })
            .collect();
        emit_data(format, &tools, &human);
        return Ok(Outcome::Ok);
    }
    let Some(script) = &args.script else {
        unreachable!("clap requires it without --tools");
    };
    let model = &setup::speech_model(args.model.as_deref()).map_err(runtime_failure)?;
    let punctuation = setup::punctuation_model(args.punctuation.as_deref());
    let mic = args.mic.as_deref().map_or_else(Vec::new, |m| {
        m.split_whitespace().map(str::to_string).collect()
    });
    let report = run_record(&Record {
        script,
        with: args.with.as_deref(),
        model,
        punctuation: punctuation.as_deref(),
        mic,
        shell: args.shell,
        url: args.url.as_deref(),
        force: args.force,
        status: args.status.as_deref(),
        quiet: args.quiet,
    })
    .map_err(Outcome::RuntimeFailure)?;
    if !args.quiet {
        emit(format, &report, &report.render());
    }
    Ok(Outcome::Ok)
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
}
