//! `teleprompt record <script>`: a shell in a pseudo-terminal, recorded
//! keystroke by keystroke as an asciicast, and the microphone recorded
//! beside it by ffmpeg. When the shell exits, the two are imported into
//! `<script>` (`crate::cmd::import`).

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::cmd::import::{run_import, Import, ImportReport, Words};
use crate::project::Project;

pub struct Record<'a> {
    pub script: &'a Path,
    pub model: &'a Path,
    /// ffmpeg's input arguments for the microphone; the platform's default
    /// input when empty.
    pub mic: Vec<String>,
    /// The program to record; `$SHELL` when empty.
    pub shell: Vec<String>,
    pub force: bool,
}

/// The rate the microphone is recorded at.
const MIC_RATE: u64 = 48_000;

pub fn run_record(r: &Record) -> Result<ImportReport, String> {
    if r.script.exists() && !r.force {
        return Err(format!(
            "{} already exists; pass --force to replace it and its lines' takes",
            r.script.display()
        ));
    }
    // Everything that could stop the import is checked before anything is
    // recorded, rather than after the author has talked for ten minutes.
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

    let voice = dir.join("voice.wav");
    let mic = Mic::start(&voice, &r.mic)?;
    eprintln!(
        "recording {} and the microphone: talk, type, and exit the shell to finish\r",
        dir.display()
    );
    let raw = RawMode::enter();
    let recorded = record_pty(
        &r.shell,
        Box::new(std::io::stdin()),
        Box::new(std::io::stdout()),
        terminal_size(),
    );
    drop(raw);
    let mic_started = mic.stop();
    let (cast, started) = recorded.map_err(|e| format!("the recording failed: {e}"))?;

    let cast_path = dir.join("session.cast");
    std::fs::write(&cast_path, cast)
        .map_err(|e| format!("cannot write {}: {e}", cast_path.display()))?;
    eprintln!("transcribing {}", voice.display());
    run_import(&Import {
        cast: &cast_path,
        voice: &voice,
        script: r.script,
        words: Words::Model(r.model),
        offset_ms: signed_ms(mic_started, started),
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

/// Runs `shell` in a pseudo-terminal of `size` (columns, rows) until it
/// exits, passing `input` to it and its output to `output`, and returns
/// the session as an asciicast (version 2, with input) and when it began.
pub fn record_pty(
    shell: &[String],
    input: Box<dyn Read + Send>,
    mut output: Box<dyn Write + Send>,
    size: (u16, u16),
) -> std::io::Result<(String, Instant)> {
    let pty = native_pty_system()
        .openpty(PtySize {
            cols: size.0,
            rows: size.1,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(std::io::Error::other)?;
    let mut command = match shell.split_first() {
        Some((program, args)) => {
            let mut c = CommandBuilder::new(program);
            c.args(args);
            c
        }
        None => CommandBuilder::new_default_prog(),
    };
    command.cwd(std::env::current_dir()?);
    let mut child = pty
        .slave
        .spawn_command(command)
        .map_err(std::io::Error::other)?;
    // The shell holds the only end of the terminal now, so reading ours
    // ends when it exits.
    drop(pty.slave);
    let started = Instant::now();
    let events = Arc::new(Mutex::new(Vec::<(f64, &'static str, String)>::new()));
    let at = move || started.elapsed().as_secs_f64();

    let mut reader = pty
        .master
        .try_clone_reader()
        .map_err(std::io::Error::other)?;
    let mut writer = pty.master.take_writer().map_err(std::io::Error::other)?;
    let log = Arc::clone(&events);
    std::thread::spawn(move || {
        let mut input = input;
        let mut buf = [0u8; 1024];
        let mut text = Utf8::default();
        while let Ok(n @ 1..) = input.read(&mut buf) {
            let s = text.push(&buf[..n]);
            if !s.is_empty() {
                lock(&log).push((at(), "i", s));
            }
            if writer
                .write_all(&buf[..n])
                .and_then(|()| writer.flush())
                .is_err()
            {
                break;
            }
        }
    });
    let log = Arc::clone(&events);
    let out = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut text = Utf8::default();
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            let s = text.push(&buf[..n]);
            if !s.is_empty() {
                lock(&log).push((at(), "o", s));
            }
            if output
                .write_all(&buf[..n])
                .and_then(|()| output.flush())
                .is_err()
            {
                break;
            }
        }
    });
    child.wait()?;
    let _ = out.join();

    let mut cast =
        serde_json::json!({ "version": 2, "width": size.0, "height": size.1 }).to_string();
    cast.push('\n');
    for (t, kind, data) in lock(&events).iter() {
        cast.push_str(&serde_json::json!([(t * 1000.0).round() / 1000.0, kind, data]).to_string());
        cast.push('\n');
    }
    Ok((cast, started))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Bytes into text, holding back a character split between two reads.
#[derive(Default)]
struct Utf8 {
    pending: Vec<u8>,
}

impl Utf8 {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.pending) {
            Ok(_) => self.pending.len(),
            // An incomplete character at the end waits for the next read;
            // anything else invalid is replaced.
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => self.pending.len(),
        };
        let rest = self.pending.split_off(valid);
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending = rest;
        text
    }
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
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error"])
            .args(&input)
            .args(["-ac", "1", "-ar", &MIC_RATE.to_string(), "-y"])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
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
                let mut err = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut err);
                }
                return Err(format!(
                    "ffmpeg could not record the microphone ({}): {}\n  \
                     pass ffmpeg's input with --mic, e.g. --mic \"-f alsa -i default\"",
                    input.join(" "),
                    err.trim()
                ));
            }
            if Instant::now() > deadline {
                let _ = child.kill();
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

/// ffmpeg's input for the platform's default microphone.
fn default_mic() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["-f", "avfoundation", "-i", ":0"]
    } else {
        &["-f", "pulse", "-i", "default"]
    }
}

/// Our terminal in raw mode while the shell runs, so each key goes to it
/// as pressed; restored when dropped. Nothing when stdin is no terminal.
struct RawMode {
    saved: Option<rustix::termios::Termios>,
}

impl RawMode {
    fn enter() -> RawMode {
        let stdin = std::io::stdin();
        let saved = rustix::termios::tcgetattr(&stdin).ok();
        if let Some(saved) = &saved {
            let mut raw = saved.clone();
            raw.make_raw();
            let _ = rustix::termios::tcsetattr(&stdin, rustix::termios::OptionalActions::Now, &raw);
        }
        RawMode { saved }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if let Some(saved) = &self.saved {
            let stdin = std::io::stdin();
            let _ =
                rustix::termios::tcsetattr(&stdin, rustix::termios::OptionalActions::Now, saved);
        }
    }
}

/// Our terminal's columns and rows, for the shell's.
fn terminal_size() -> (u16, u16) {
    rustix::termios::tcgetwinsize(std::io::stdout())
        .map_or((80, 24), |w| (w.ws_col.max(20), w.ws_row.max(5)))
}
