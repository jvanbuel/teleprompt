//! `teleprompt prompt <script>`: a prompter that follows the reader's voice.
//!
//! The prompter itself is `teleprompt-prompter`; this is its REST API and
//! the page that drives it. Hand-rolled over `TcpListener` like `serve`:
//! one browser, on localhost.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

use teleprompt_listen::Recognizer;
use teleprompt_prompter::{Prompt, Reached, Script, Session, LISTEN_RATE};

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
        shots: teleprompt_prompter::shot_cues(&compiled),
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

/// Serves `prompt` on `listener` until the process ends: the page, and
/// the [`Session`] as a REST API.
///
/// | route | does |
/// |---|---|
/// | `GET /script.json` | the lines, whether each is recorded, and each shot with its cue and clip URL |
/// | `POST /start?from=N` | a new take from line N (default 0) |
/// | `POST /listen?rate=HZ` | little-endian f32 mono samples (default 16 kHz); where the reader is |
/// | `POST /stop` | ends the take; the ids of the lines kept |
/// | `GET /clips/<key>.mp4` | a shot's captured clip |
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
        let (status, kind, body) = route(&mut session, &request);
        respond(&mut stream, status, kind, &body)?;
    }
    Ok(())
}

type Response = (&'static str, &'static str, Vec<u8>);

fn route<R: Recognizer>(session: &mut Session<R>, request: &Request) -> Response {
    let (path, query) = request
        .path
        .split_once('?')
        .unwrap_or((request.path.as_str(), ""));
    match (request.method.as_str(), path) {
        ("POST", "/listen") => {
            let rate = param(query, "rate").unwrap_or(LISTEN_RATE);
            json(reached(session.listen(&samples(&request.body), rate)))
        }
        ("POST", "/start") => json(reached(session.start(param(query, "from").unwrap_or(0)))),
        ("POST", "/stop") => match session.stop() {
            Ok(saved) => json(serde_json::json!({ "saved": saved })),
            Err(e) => failed(e),
        },
        ("GET", "/") => ("200 OK", "text/html; charset=utf-8", PAGE.into()),
        ("GET", "/favicon.ico") => ("204 No Content", "text/plain", Vec::new()),
        ("GET", "/script.json") => json(script(session.script())),
        ("GET", path) => match path
            .strip_prefix("/clips/")
            .and_then(|name| name.strip_suffix(".mp4"))
            .and_then(|key| session.clip(key))
        {
            Some(clip) => match std::fs::read(clip) {
                Ok(bytes) => ("200 OK", "video/mp4", bytes),
                Err(e) => failed(e),
            },
            None => not_found(),
        },
        _ => not_found(),
    }
}

fn reached(r: Reached) -> serde_json::Value {
    serde_json::json!({ "line": r.at.line, "word": r.at.word, "play": r.play })
}

fn script(s: Script) -> serde_json::Value {
    let shots: Vec<_> = s
        .shots
        .iter()
        .map(|shot| {
            serde_json::json!({
                "shot": shot.shot,
                "at": { "line": shot.at.line, "word": shot.at.word },
                "clip": shot.clip.as_ref().map(|_| format!("/clips/{}.mp4", shot.capture_key)),
            })
        })
        .collect();
    let lines: Vec<_> = s.lines.iter().map(|l| &l.text).collect();
    let recorded: Vec<_> = s.lines.iter().map(|l| l.recorded).collect();
    serde_json::json!({ "lines": lines, "recorded": recorded, "shots": shots })
}

fn json(value: serde_json::Value) -> Response {
    ("200 OK", "application/json", value.to_string().into_bytes())
}

fn failed(e: std::io::Error) -> Response {
    (
        "500 Internal Server Error",
        "text/plain",
        e.to_string().into(),
    )
}

fn not_found() -> Response {
    ("404 Not Found", "text/plain", b"not found".to_vec())
}

/// `name`'s value in a query string.
fn param<T: std::str::FromStr>(query: &str, name: &str) -> Option<T> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .and_then(|(_, value)| value.parse().ok())
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
