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
use teleprompt_listen::{Cues, Follower, Position, Recognizer, TakeLog};
use teleprompt_voice::takes::Takes;
use teleprompt_voice::{Pcm, Resampler};

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
        ids: compiled
            .narration
            .iter()
            .map(|n| n.line_id.clone())
            .collect(),
        lines: compiled.narration.into_iter().map(|n| n.text).collect(),
        clips: project.caches().clips(),
        takes: project.takes_dir(),
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
    /// Each line's id, which names its take.
    pub ids: Vec<String>,
    /// Every shot, in script order.
    pub shots: Vec<ShotCue>,
    /// Where captured clips are, named by capture key.
    pub clips: PathBuf,
    /// Where takes are recorded to.
    pub takes: PathBuf,
}

/// The rate the recognizer listens at. The page sends the microphone at
/// its own rate, which is what a take keeps.
const LISTEN_RATE: u32 = 16_000;

/// Serves `prompt` on `listener` until the process ends.
pub fn prompt_on<R: Recognizer>(
    listener: TcpListener,
    prompt: Prompt,
    recognizer: R,
) -> std::io::Result<()> {
    let mut session = Session::new(prompt, recognizer)?;
    for stream in listener.incoming() {
        let mut stream = stream?;
        let Ok(request) = read_request(&mut stream) else {
            continue;
        };
        let (path, query) = request
            .path
            .split_once('?')
            .unwrap_or((request.path.as_str(), ""));
        let (status, kind, body) = match (request.method.as_str(), path) {
            ("POST", "/listen") => {
                let rate = param(query, "rate").unwrap_or(LISTEN_RATE);
                json(session.listen(&samples(&request.body), rate))
            }
            ("POST", "/start") => json(session.start(param(query, "from").unwrap_or(0))),
            ("POST", "/stop") => match session.stop() {
                Ok(saved) => json(saved),
                Err(e) => (
                    "500 Internal Server Error",
                    "text/plain",
                    e.to_string().into(),
                ),
            },
            ("GET", "/") => ("200 OK", "text/html; charset=utf-8", PAGE.into()),
            ("GET", path) if session.clips.contains_key(path) => {
                match std::fs::read(&session.clips[path]) {
                    Ok(clip) => ("200 OK", "video/mp4", clip),
                    Err(e) => (
                        "500 Internal Server Error",
                        "text/plain",
                        e.to_string().into(),
                    ),
                }
            }
            ("GET", "/favicon.ico") => ("204 No Content", "text/plain", Vec::new()),
            ("GET", "/script.json") => json(session.script()),
            _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
        };
        respond(&mut stream, status, kind, &body)?;
    }
    Ok(())
}

fn json(value: serde_json::Value) -> (&'static str, &'static str, Vec<u8>) {
    ("200 OK", "application/json", value.to_string().into_bytes())
}

/// `name`'s value in a query string.
fn param<T: std::str::FromStr>(query: &str, name: &str) -> Option<T> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .and_then(|(_, value)| value.parse().ok())
}

/// The prompter's state between requests.
struct Session<R> {
    prompt: Prompt,
    follower: Follower<R>,
    cues: Cues,
    takes: Takes,
    /// Request path to file, for the clips of cued shots that exist.
    clips: BTreeMap<String, PathBuf>,
    at: Position,
    /// The microphone's rate, and its conversion to [`LISTEN_RATE`].
    resampler: Option<(u32, Resampler)>,
    /// The take under way, from `/start` to `/stop`.
    take: Option<Take>,
}

struct Take {
    rate: u32,
    audio: Vec<f32>,
    log: TakeLog,
}

impl<R: Recognizer> Session<R> {
    fn new(prompt: Prompt, recognizer: R) -> std::io::Result<Self> {
        let texts: Vec<&str> = prompt.lines.iter().map(String::as_str).collect();
        let follower = Follower::new(recognizer, &texts);
        let cues = Cues::new(prompt.shots.iter().map(|s| s.at).collect());
        let clips = prompt
            .shots
            .iter()
            .map(|s| format!("{}.mp4", s.capture_key))
            .map(|name| (format!("/clips/{name}"), prompt.clips.join(name)))
            .filter(|(_, path)| path.is_file())
            .collect();
        Ok(Self {
            takes: Takes::load(&prompt.takes)?,
            prompt,
            follower,
            cues,
            clips,
            at: Position { line: 0, word: 0 },
            resampler: None,
            take: None,
        })
    }

    /// A new take from line `from`; any take not stopped is dropped.
    fn start(&mut self, from: usize) -> serde_json::Value {
        self.at = Position {
            line: from,
            word: 0,
        };
        self.follower.restart_at(from);
        self.cues.restart_at(self.at);
        self.resampler = None;
        let mut log = TakeLog::new(from);
        log.heard(self.at, 0);
        self.take = Some(Take {
            rate: LISTEN_RATE,
            audio: Vec::new(),
            log,
        });
        self.heard()
    }

    /// Microphone samples at `rate`: kept for the take, and heard.
    fn listen(&mut self, samples: &[f32], rate: u32) -> serde_json::Value {
        if let Some(take) = &mut self.take {
            if take.audio.is_empty() {
                take.rate = rate;
            }
            take.audio.extend_from_slice(samples);
        }
        let heard = if rate == LISTEN_RATE {
            samples.to_vec()
        } else {
            if self.resampler.as_ref().is_none_or(|(r, _)| *r != rate) {
                self.resampler = Some((rate, Resampler::new(rate, LISTEN_RATE)));
            }
            let (_, resampler) = self.resampler.as_mut().expect("just set");
            resampler.push(samples)
        };
        if let Some(now) = self.follower.listen(&heard) {
            self.at = now;
            if let Some(take) = &mut self.take {
                take.log.heard(now, take.audio.len());
            }
        }
        self.heard()
    }

    /// Ends the take, keeping each line read in full as that line's take.
    fn stop(&mut self) -> std::io::Result<serde_json::Value> {
        let mut saved = Vec::new();
        if let Some(take) = self.take.take() {
            for (line, span) in take.log.lines(&take.audio, take.rate) {
                let (Some(id), Some(text)) =
                    (self.prompt.ids.get(line), self.prompt.lines.get(line))
                else {
                    continue;
                };
                let pcm = Pcm {
                    sample_rate: take.rate,
                    channels: 1,
                    samples: take.audio[span]
                        .iter()
                        .map(|&s| (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
                        .collect(),
                };
                self.takes.save(id, text, &pcm)?;
                saved.push(id.clone());
            }
        }
        Ok(serde_json::json!({ "saved": saved }))
    }

    /// Where the reader is, and the shots that just reached their cue.
    fn heard(&mut self) -> serde_json::Value {
        let play: Vec<&str> = self.prompt.shots[self.cues.reach(self.at)]
            .iter()
            .map(|s| s.shot.as_str())
            .collect();
        serde_json::json!({ "line": self.at.line, "word": self.at.word, "play": play })
    }

    /// The lines and whether each has a current take, and each shot with
    /// where it starts and the path of its clip, if it was captured.
    fn script(&self) -> serde_json::Value {
        let shots: Vec<_> = self
            .prompt
            .shots
            .iter()
            .map(|s| {
                let clip = format!("/clips/{}.mp4", s.capture_key);
                serde_json::json!({
                    "shot": s.shot,
                    "at": { "line": s.at.line, "word": s.at.word },
                    "clip": self.clips.contains_key(&clip).then_some(clip),
                })
            })
            .collect();
        let recorded: Vec<bool> = self
            .prompt
            .ids
            .iter()
            .zip(&self.prompt.lines)
            .map(|(id, text)| self.takes.current(id, text).is_some())
            .collect();
        serde_json::json!({ "lines": self.prompt.lines, "recorded": recorded, "shots": shots })
    }
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
