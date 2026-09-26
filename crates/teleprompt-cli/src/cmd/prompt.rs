//! `teleprompt prompt <script>`: a prompter that follows the reader's voice.
//!
//! The page posts the microphone's samples to `/listen` as they come, and
//! the answer is where the reader now is. Hand-rolled over `TcpListener`
//! like `serve`: one browser, on localhost.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

use teleprompt_compile::CompileOutput;
use teleprompt_core::Hash;
use teleprompt_listen::{Cues, Follower, Position, Recognizer};

use crate::output::Outcome;
use crate::project::Project;

/// The speech model `prompt` is tested with: sherpa-onnx's streaming
/// English zipformer. The smaller 20M model misses the first words of a
/// stream. teleprompt downloads nothing, so the error naming it says where
/// to get it.
pub const MODEL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-2023-06-26.tar.bz2";

#[derive(Debug)]
pub enum PromptError {
    Validation(Vec<String>),
    Runtime(String),
}

impl From<PromptError> for Outcome {
    fn from(e: PromptError) -> Self {
        match e {
            PromptError::Validation(errors) => Self::ValidationError(errors),
            PromptError::Runtime(message) => Self::RuntimeFailure(message),
        }
    }
}

/// Serves a prompter for `script`'s narration on loopback, following the
/// reader with the speech model in `model`.
#[cfg(feature = "listen")]
pub fn run_prompt(
    project: &Project,
    script: &std::path::Path,
    locale: &str,
    port: u16,
    model: Option<&std::path::Path>,
) -> Result<(), PromptError> {
    let dir = model.ok_or_else(|| {
        PromptError::Runtime(format!(
            "`prompt` needs a speech model: download and unpack {MODEL}, \
             then pass its directory with --model"
        ))
    })?;
    let recognizer =
        teleprompt_listen_sherpa::SherpaRecognizer::new(dir).map_err(PromptError::Runtime)?;
    let (compiled, _) = crate::cmd::check::compile_script(project, script, locale)
        .map_err(PromptError::Validation)?;
    let prompt = Prompt {
        shots: shot_cues(&compiled),
        lines: compiled.narration.into_iter().map(|n| n.text).collect(),
        clips: project.caches().clips(),
    };
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .map_err(|e| PromptError::Runtime(format!("cannot listen on port {port}: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| PromptError::Runtime(e.to_string()))?;
    eprintln!("prompting at http://{addr}/ — open it, click, and read");
    prompt_on(listener, prompt, recognizer).map_err(|e| PromptError::Runtime(e.to_string()))
}

/// A build without the recognizer cannot follow anyone; it says how to get
/// one.
#[cfg(not(feature = "listen"))]
pub fn run_prompt(
    _project: &Project,
    _script: &std::path::Path,
    _locale: &str,
    _port: u16,
    _model: Option<&std::path::Path>,
) -> Result<(), PromptError> {
    Err(PromptError::Runtime(
        "this teleprompt was built without a speech recognizer; \
         rebuild it with `--features listen`"
            .to_string(),
    ))
}

const PAGE: &str = include_str!("prompt.html");

/// A shot, and the point in the script at which the reader's voice starts
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotCue {
    pub shot: String,
    /// Names the shot's clip among the captured ones.
    pub capture_key: Hash,
    pub at: Position,
}

/// Where each shot starts, as the video would start it, but counted in
/// words said rather than milliseconds: an action with no line of its own
/// follows the lines before it, one that starts after its line ends waits
/// for the line to be said, and one that overlaps its line starts on the
/// word the timeline puts it at.
pub fn shot_cues(compiled: &CompileOutput) -> Vec<ShotCue> {
    let mut cues = Vec::new();
    // How many lines the timeline has begun.
    let mut line = 0;
    for entry in &compiled.timeline.entries {
        if let Some(action) = &entry.action {
            let at = match (&entry.narration, compiled.narration.get(line)) {
                (Some(n), _) if action.start_ms >= n.start_ms + n.duration_ms => Position {
                    line: line + 1,
                    word: 0,
                },
                (Some(n), Some(detail)) => Position {
                    line,
                    word: word_at(
                        &detail.text,
                        action.start_ms.saturating_sub(n.start_ms),
                        n.duration_ms,
                    ) + 1,
                },
                _ => Position { line, word: 0 },
            };
            cues.push(ShotCue {
                shot: action.shot.clone(),
                capture_key: action.capture_key,
                at,
            });
        }
        if entry.narration.is_some() {
            line += 1;
        }
    }
    cues
}

/// The word being said `offset_ms` into a line `duration_ms` long, spread
/// over its characters as a cue without word timings is
/// (`docs/design.md#cues`), so that inverting it lands on the cue's word.
fn word_at(text: &str, offset_ms: u64, duration_ms: u64) -> usize {
    if duration_ms == 0 {
        return 0;
    }
    let chars = text.chars().count() as f64;
    let at = (offset_ms as f64 * chars / duration_ms as f64).round() as usize;
    // Words are counted as the aligner counts them: split at whitespace.
    let mut after_space = true;
    let begun = text
        .chars()
        .enumerate()
        .filter(|&(_, c)| {
            let starts = after_space && !c.is_whitespace();
            after_space = c.is_whitespace();
            starts
        })
        .take_while(|&(i, _)| i <= at)
        .count();
    begun.saturating_sub(1)
}

/// What the prompter shows and plays.
pub struct Prompt {
    /// The narration, a line per paragraph.
    pub lines: Vec<String>,
    /// Every shot, in script order.
    pub shots: Vec<ShotCue>,
    /// Where captured clips are, named by capture key.
    pub clips: PathBuf,
}

/// Serves `prompt` on `listener` until the process ends.
pub fn prompt_on<R: Recognizer>(
    listener: TcpListener,
    prompt: Prompt,
    recognizer: R,
) -> std::io::Result<()> {
    let texts: Vec<&str> = prompt.lines.iter().map(String::as_str).collect();
    let mut follower = Follower::new(recognizer, &texts);
    let mut cues = Cues::new(prompt.shots.iter().map(|s| s.at).collect());
    let clips: BTreeMap<String, PathBuf> = prompt
        .shots
        .iter()
        .map(|s| format!("{}.mp4", s.capture_key))
        .map(|name| (format!("/clips/{name}"), prompt.clips.join(name)))
        .filter(|(_, path)| path.is_file())
        .collect();
    let script = script_json(&prompt, &clips);
    let mut at = Position { line: 0, word: 0 };
    for stream in listener.incoming() {
        let mut stream = stream?;
        let Ok(request) = read_request(&mut stream) else {
            continue;
        };
        let (status, body) = match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/listen") => {
                if let Some(now) = follower.listen(&samples(&request.body)) {
                    at = now;
                }
                ("200 OK", heard_json(at, &prompt.shots[cues.reach(at)]))
            }
            ("POST", "/start") => {
                follower.restart();
                cues.restart();
                at = Position { line: 0, word: 0 };
                ("200 OK", heard_json(at, &prompt.shots[cues.reach(at)]))
            }
            ("GET", "/") => {
                respond(
                    &mut stream,
                    "200 OK",
                    "text/html; charset=utf-8",
                    PAGE.as_bytes(),
                )?;
                continue;
            }
            ("GET", path) if clips.contains_key(path) => {
                match std::fs::read(&clips[path]) {
                    Ok(clip) => respond(&mut stream, "200 OK", "video/mp4", &clip)?,
                    Err(e) => respond(
                        &mut stream,
                        "500 Internal Server Error",
                        "text/plain",
                        e.to_string().as_bytes(),
                    )?,
                }
                continue;
            }
            ("GET", "/favicon.ico") => ("204 No Content", String::new()),
            ("GET", "/script.json") => ("200 OK", script.clone()),
            _ => ("404 Not Found", "not found".to_string()),
        };
        respond(&mut stream, status, "application/json", body.as_bytes())?;
    }
    Ok(())
}

/// The lines, and each shot with where it starts and the path of its clip,
/// if it was captured.
fn script_json(prompt: &Prompt, clips: &BTreeMap<String, PathBuf>) -> String {
    let shots: Vec<_> = prompt
        .shots
        .iter()
        .map(|s| {
            let clip = format!("/clips/{}.mp4", s.capture_key);
            serde_json::json!({
                "shot": s.shot,
                "at": { "line": s.at.line, "word": s.at.word },
                "clip": clips.contains_key(&clip).then_some(clip),
            })
        })
        .collect();
    serde_json::json!({ "lines": prompt.lines, "shots": shots }).to_string()
}

/// Where the reader is, and the shots that just reached their cue.
fn heard_json(at: Position, play: &[ShotCue]) -> String {
    let play: Vec<&str> = play.iter().map(|s| s.shot.as_str()).collect();
    serde_json::json!({ "line": at.line, "word": at.word, "play": play }).to_string()
}

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    let mut reader = BufReader::new(&*stream);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request { method, path, body })
}

/// Little-endian f32 samples, as the page sends them.
fn samples(body: &[u8]) -> Vec<f32> {
    body.chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}
