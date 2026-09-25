//! `teleprompt prompt <script>`: a prompter that follows the reader's voice.
//!
//! The page posts the microphone's samples to `/listen` as they come, and
//! the answer is where the reader now is. Hand-rolled over `TcpListener`
//! like `serve`: one browser, on localhost.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

use teleprompt_listen::{Follower, Position, Recognizer};

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
    let lines = compiled.narration.into_iter().map(|n| n.text).collect();
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .map_err(|e| PromptError::Runtime(format!("cannot listen on port {port}: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| PromptError::Runtime(e.to_string()))?;
    eprintln!("prompting at http://{addr}/ — open it, click, and read");
    prompt_on(listener, lines, recognizer).map_err(|e| PromptError::Runtime(e.to_string()))
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

/// Serves the prompter for `lines` on `listener` until the process ends.
pub fn prompt_on<R: Recognizer>(
    listener: TcpListener,
    lines: Vec<String>,
    recognizer: R,
) -> std::io::Result<()> {
    let texts: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut follower = Follower::new(recognizer, &texts);
    let script = serde_json::json!({ "lines": lines }).to_string();
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
                ("200 OK", position_json(at))
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
            ("POST", "/start") => {
                follower.restart();
                at = Position { line: 0, word: 0 };
                ("200 OK", position_json(at))
            }
            ("GET", "/favicon.ico") => ("204 No Content", String::new()),
            ("GET", "/script.json") => ("200 OK", script.clone()),
            _ => ("404 Not Found", "not found".to_string()),
        };
        respond(&mut stream, status, "application/json", body.as_bytes())?;
    }
    Ok(())
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

fn position_json(p: Position) -> String {
    format!(r#"{{"line":{},"word":{}}}"#, p.line, p.word)
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
