//! `teleprompt prompt <script>`: a prompter that follows the reader's voice.
//!
//! The prompter itself is `teleprompt-prompter`; this is its API, version
//! 1, and the page that drives it: HTTP for the script and clips, and a
//! WebSocket for the session. Hand-rolled over `TcpListener` like the preview:
//! one reader, on localhost.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use teleprompt_listen::Recognizer;
use teleprompt_prompter::{Position, Prompt, Reached, Script, Session, LISTEN_RATE};
use tungstenite::handshake::derive_accept_key;
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

use crate::cmd::voicing::Voicing;
use crate::output::{Format, Outcome};
use crate::project::Project;
use teleprompt_core::edit::Edit;

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

/// How a prompter follows its script: by ear, with a speech model (the
/// one named, or none found), or read by the script's voice.
pub enum Ear<'a> {
    Model(Option<&'a std::path::Path>),
    Voice,
}

/// Serves a prompter for `script`'s narration on loopback, following the
/// reader with the speech model in `ear`, or reading it with its voice.
pub fn run_prompt(
    project: &Project,
    script: &std::path::Path,
    locale: &str,
    port: u16,
    ear: Ear<'_>,
    format: Format,
) -> Result<(), PromptError> {
    match ear {
        Ear::Voice => serve(
            project,
            script,
            locale,
            port,
            format,
            teleprompt_listen::Deaf,
            false,
        ),
        Ear::Model(model) => {
            let recognizer = recognizer(model)?;
            serve(project, script, locale, port, format, recognizer, true)
        }
    }
}

#[cfg(feature = "listen")]
fn recognizer(
    model: Option<&std::path::Path>,
) -> Result<teleprompt_listen_sherpa::SherpaRecognizer, PromptError> {
    let dir = model.ok_or_else(|| {
        PromptError::Runtime(format!(
            "`prompt` needs a speech model: `teleprompt setup speech-model` installs \
             one, or download and unpack {MODEL} and pass its directory with --model"
        ))
    })?;
    teleprompt_listen_sherpa::SherpaRecognizer::new(dir).map_err(PromptError::Runtime)
}

/// A build without the recognizer cannot follow anyone; it says how to get
/// one, or to let the voice read.
#[cfg(not(feature = "listen"))]
fn recognizer(_model: Option<&std::path::Path>) -> Result<teleprompt_listen::Deaf, PromptError> {
    Err(PromptError::Runtime(
        "this teleprompt was built without a speech recognizer; \
         rebuild it with `--features listen`, or pass --voice to have the \
         script's voice read it"
            .to_string(),
    ))
}

fn serve<R: Recognizer + Send + 'static>(
    project: &Project,
    script: &std::path::Path,
    locale: &str,
    port: u16,
    format: Format,
    recognizer: R,
    listens: bool,
) -> Result<(), PromptError> {
    let prompt = prompt_of(project, script, locale).map_err(PromptError::Validation)?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .map_err(|e| PromptError::Runtime(format!("cannot listen on port {port}: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| PromptError::Runtime(e.to_string()))?;
    eprintln!("prompting at http://{addr}/ — open it, click, and read");
    if format == Format::Json {
        // An app that launched the command reads where to connect from this.
        println!("{}", listening_event(addr));
        std::io::stdout()
            .flush()
            .map_err(|e| PromptError::Runtime(e.to_string()))?;
    }
    let mut edits = edits_of(project, script, locale);
    edits.listens = listens;
    prompt_watching(listener, prompt, recognizer, Some(edits))
        .map_err(|e| PromptError::Runtime(e.to_string()))
}

/// What the prompter shows of `script`, as it now reads.
fn prompt_of(
    project: &Project,
    script: &std::path::Path,
    locale: &str,
) -> Result<Prompt, Vec<String>> {
    let (compiled, _) = crate::cmd::check::compile_script(project, script, locale)?;
    Ok(Prompt {
        name: script
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        shots: teleprompt_prompter::shot_cues(&compiled),
        ids: compiled
            .narration
            .iter()
            .map(|n| n.line_id.clone())
            .collect(),
        lines: compiled.narration.into_iter().map(|n| n.text).collect(),
        clips: project.caches().clips(),
        takes: project.takes_dir(),
    })
}

/// The script again whenever its file has changed since last asked, for
/// one edited while it is being read: a shot moved or stretched, a line
/// reworded. `None` when it has not changed, or does not compile.
pub fn reload_on_edit(project: &Project, script: &std::path::Path, locale: &str) -> Reload {
    let seen = Mutex::new(crate::project::fingerprint(script));
    let (project, script, locale) = (project.clone(), script.to_path_buf(), locale.to_string());
    Box::new(move || {
        let now = crate::project::fingerprint(&script);
        let mut seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
        if now == *seen {
            return None;
        }
        *seen = now;
        prompt_of(&project, &script, &locale).ok()
    })
}

/// The script anew, if it has changed.
pub type Reload = Box<dyn Fn() -> Option<Prompt> + Send + Sync>;

/// Rewords a line to what its take was heard to say, or says why not.
pub type KeepSaid = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Makes an edit to the script, or says why not.
pub type EditScript = Box<dyn Fn(&Edit) -> Result<(), String> + Send + Sync>;

/// What a prompter reading a script file can do to it.
pub struct Edits {
    pub reload: Reload,
    pub keep_said: KeepSaid,
    pub edit: EditScript,
    /// How its lines sound when its voice reads them; none for a script
    /// that is not a project's file.
    pub voice: Option<Voicing>,
    /// Whether it follows a reader by ear; if not, its voice reads.
    pub listens: bool,
}

/// `script`'s edits: reloaded when changed, and a line reworded as
/// `teleprompt edit <script> said <line>` does.
pub fn edits_of(project: &Project, script: &std::path::Path, locale: &str) -> Edits {
    let (keeper, path) = (project.clone(), script.to_path_buf());
    let (editor, edited) = (project.clone(), script.to_path_buf());
    Edits {
        reload: reload_on_edit(project, script, locale),
        keep_said: Box::new(move |line| {
            crate::cmd::edit::run_said(&keeper, &path, line)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }),
        edit: Box::new(move |edit| {
            crate::cmd::edit::run_edit(&editor, &edited, edit)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }),
        voice: Some(Voicing::new(project, script, locale)),
        listens: true,
    }
}

/// Where the command listens, for `--format json`: the API's origin and
/// the path its version is served under.
pub fn listening_event(addr: SocketAddr) -> serde_json::Value {
    serde_json::json!({ "event": "listening", "url": format!("http://{addr}"), "api": "/api/v1" })
}

/// The prompters' typeface (`apps/fonts`), Latin, for the page.
pub(crate) const FONT: &[u8] = include_bytes!("prompt-font.woff2");
pub(crate) const FONT_PATH: &str = "/fonts/atkinson-hyperlegible-next.woff2";
/// The app icon, `apps/icons/teleprompt.svg`, as the pages' tab icon.
pub(crate) const ICON: &[u8] = include_bytes!("prompt-icon.svg");
pub(crate) const ICON_PATH: &str = "/icon.svg";

const PAGE: &str = include_str!("prompt.html");

/// Serves `prompt` on `listener` until the process ends: the page, and
/// the [`Session`] as API version 1 (`docs/design.md#prompter-api-version-1`):
/// `GET /api/v1/script`, `GET /api/v1/clips/<key>.mp4`, and the session as
/// a WebSocket at `GET /api/v1/session`.
pub fn prompt_on<R: Recognizer + Send + 'static>(
    listener: TcpListener,
    prompt: Prompt,
    recognizer: R,
) -> std::io::Result<()> {
    prompt_watching(listener, prompt, recognizer, None)
}

/// [`prompt_on`] for a script file: placing the shots again when `edits`
/// reloads them, asked each time the script is fetched, which a client
/// does after an edit, and keeping what a take said when asked.
pub fn prompt_watching<R: Recognizer + Send + 'static>(
    listener: TcpListener,
    prompt: Prompt,
    recognizer: R,
    edits: Option<Edits>,
) -> std::io::Result<()> {
    let server = Arc::new(Server {
        session: Mutex::new(Session::new(prompt, recognizer)?),
        open: AtomicBool::new(false),
        edits,
    });
    for stream in listener.incoming() {
        // One connection that fails to arrive ends no one's session.
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!("warning: dropped connection: {e}");
                continue;
            }
        };
        let server = server.clone();
        // A session socket stays open, so each connection has a thread.
        std::thread::spawn(move || {
            let _ = server.handle(stream);
        });
    }
    Ok(())
}

struct Server<R> {
    session: Mutex<Session<R>>,
    /// Whether a session socket is open.
    open: AtomicBool,
    edits: Option<Edits>,
}

type Response = (&'static str, &'static str, Vec<u8>);

impl<R: Recognizer> Server<R> {
    fn session(&self) -> MutexGuard<'_, Session<R>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn handle(&self, mut stream: TcpStream) -> std::io::Result<()> {
        let request = read_request(&mut stream)?;
        let from = (request.header("host"), request.header("origin"));
        if let Some(why) = crate::loopback::refused(from.0, from.1) {
            return respond(
                &mut stream,
                "403 Forbidden",
                "text/plain",
                &[],
                why.as_bytes(),
            );
        }
        if request.method == "GET" && request.path == "/api/v1/session" {
            return self.open_session(stream, &request);
        }
        let (status, kind, body) = self.route(&request);
        respond(&mut stream, status, kind, &[], &body)
    }

    fn route(&self, request: &Request) -> Response {
        if request.method != "GET" {
            return not_found();
        }
        let (path, query) = request
            .path
            .split_once('?')
            .unwrap_or((request.path.as_str(), ""));
        if let Some(id) = path
            .strip_prefix("/api/v1/voice/")
            .and_then(|name| name.strip_suffix(".wav"))
        {
            let fresh = query.split('&').any(|q| q == "fresh=1");
            let voice = self.edits.as_ref().and_then(|e| e.voice.as_ref());
            return match voice.map(|v| v.audio(id, fresh)) {
                Some(Ok(Some(wav))) => ("200 OK", "audio/wav", wav),
                Some(Err(e)) => failed(std::io::Error::other(e)),
                Some(Ok(None)) | None => not_found(),
            };
        }
        match path {
            "/" => ("200 OK", "text/html; charset=utf-8", PAGE.into()),
            "/favicon.ico" => ("204 No Content", "text/plain", Vec::new()),
            FONT_PATH => ("200 OK", "font/woff2", FONT.to_vec()),
            ICON_PATH => ("200 OK", "image/svg+xml", ICON.to_vec()),
            "/api/v1/script" => {
                // Compiled outside the lock, which the session needs.
                let edited = self.edits.as_ref().and_then(|e| (e.reload)());
                let mut session = self.session();
                if let Some(prompt) = edited {
                    session.replace(prompt);
                }
                let mut script = script(session.script());
                drop(session);
                if let Some(edits) = &self.edits {
                    voiced(&mut script, edits);
                }
                json(script)
            }
            path => match path
                .strip_prefix("/api/v1/clips/")
                .and_then(|name| name.strip_suffix(".mp4"))
                .and_then(|key| self.session().clip(key))
            {
                Some(clip) => match std::fs::read(clip) {
                    Ok(bytes) => ("200 OK", "video/mp4", bytes),
                    Err(e) => failed(e),
                },
                None => not_found(),
            },
        }
    }

    /// Upgrades `stream` to the session socket, unless one is open.
    fn open_session(&self, mut stream: TcpStream, request: &Request) -> std::io::Result<()> {
        let Some(key) = request.header("sec-websocket-key") else {
            let body = b"the session is a WebSocket";
            let upgrade = [("Upgrade", "websocket")];
            return respond(
                &mut stream,
                "426 Upgrade Required",
                "text/plain",
                &upgrade,
                body,
            );
        };
        if self.open.swap(true, Ordering::SeqCst) {
            let body = b"a session is already open";
            return respond(&mut stream, "409 Conflict", "text/plain", &[], body);
        }
        // Let go when the session ends, a panic in it included.
        struct Open<'a>(&'a AtomicBool);
        impl Drop for Open<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _open = Open(&self.open);
        self.run_session(stream, key)
    }

    fn run_session(&self, mut stream: TcpStream, key: &str) -> std::io::Result<()> {
        write!(
            stream,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            derive_accept_key(key.as_bytes())
        )?;
        let mut ws = WebSocket::from_raw_socket(stream, Role::Server, None);
        let mut rate = LISTEN_RATE;
        let mut at = None;
        loop {
            let answer = match ws.read() {
                Ok(Message::Text(text)) => self.command(text.as_str(), &mut rate, &mut at),
                Ok(Message::Binary(audio)) => {
                    let reached = self.session().listen(&samples(&audio), rate);
                    let news = at != Some(reached.at) || !reached.play.is_empty();
                    at = Some(reached.at);
                    news.then(|| reached_json(&reached))
                }
                Ok(Message::Close(_))
                | Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    return Ok(())
                }
                Ok(_) => None,
                Err(e) => return Err(std::io::Error::other(e)),
            };
            if let Some(answer) = answer {
                ws.send(Message::text(answer.to_string()))
                    .map_err(std::io::Error::other)?;
            }
        }
    }

    /// A JSON message from the client, and the answer to it.
    fn command(
        &self,
        text: &str,
        rate: &mut u32,
        at: &mut Option<Position>,
    ) -> Option<serde_json::Value> {
        let message: serde_json::Value = match serde_json::from_str(text) {
            Ok(message) => message,
            Err(e) => return Some(error(format!("not JSON: {e}"))),
        };
        match message["type"].as_str() {
            Some("start") if !self.edits.as_ref().is_none_or(|e| e.listens) => Some(error(
                "this prompter reads the script with its voice; it does not listen".into(),
            )),
            Some("start") => {
                let from = message["from"].as_u64().unwrap_or(0) as usize;
                *rate = message["rate"]
                    .as_u64()
                    .and_then(|r| u32::try_from(r).ok())
                    .filter(|&r| r > 0)
                    .unwrap_or(LISTEN_RATE);
                let reached = self.session().start(from);
                *at = Some(reached.at);
                Some(reached_json(&reached))
            }
            Some("stop") => Some(match self.session().stop() {
                Ok(saved) => serde_json::json!({ "type": "stopped", "saved": saved }),
                Err(e) => error(e.to_string()),
            }),
            Some("discard") => {
                self.session().discard();
                Some(serde_json::json!({ "type": "discarded" }))
            }
            Some("undo") => Some(match self.session().undo() {
                Ok(lines) => serde_json::json!({ "type": "undone", "lines": lines }),
                Err(e) => error(e.to_string()),
            }),
            Some("keep_said") => {
                let line = message["line"].as_str().unwrap_or_default();
                Some(match &self.edits {
                    Some(edits) => match (edits.keep_said)(line) {
                        Ok(()) => serde_json::json!({ "type": "kept_said", "line": line }),
                        Err(e) => error(e),
                    },
                    None => error("this prompter has no script file to reword".into()),
                })
            }
            Some(kind @ ("reword" | "instruct")) => {
                let line = message["line"].as_str().unwrap_or_default();
                let text = message["text"].as_str().map(str::to_string);
                let edit = match (kind, text) {
                    ("reword", Some(text)) => Edit::Reword {
                        line: line.into(),
                        text,
                    },
                    ("reword", None) => return Some(error("a reword needs its text".into())),
                    (_, text) => Edit::Instruct {
                        line: line.into(),
                        text: text.filter(|t| !t.trim().is_empty()),
                    },
                };
                Some(match &self.edits {
                    Some(edits) => match (edits.edit)(&edit) {
                        Ok(()) => serde_json::json!({ "type": "edited", "line": line }),
                        Err(e) => error(e),
                    },
                    None => error("this prompter has no script file to edit".into()),
                })
            }
            _ => Some(error(format!("not a message this server knows: {text}"))),
        }
    }
}

fn reached_json(r: &Reached) -> serde_json::Value {
    serde_json::json!({ "type": "reached", "line": r.at.line, "word": r.at.word, "play": r.play })
}

fn error(message: String) -> serde_json::Value {
    serde_json::json!({ "type": "error", "message": message })
}

fn script(s: Script) -> serde_json::Value {
    let lines: Vec<_> = s
        .lines
        .iter()
        .map(|l| {
            let diff = l.said.as_deref().map_or_else(Vec::new, |said| {
                teleprompt_core::said::diff(&l.text, said)
            });
            serde_json::json!({ "id": l.id, "text": l.text, "recorded": l.recorded, "stale": l.stale, "said": l.said, "said_diff": diff })
        })
        .collect();
    let shots: Vec<_> = s
        .shots
        .iter()
        .map(|shot| {
            serde_json::json!({
                "shot": shot.shot,
                "at": { "line": shot.at.line, "word": shot.at.word },
                "clip": shot.clip.as_ref().map(|_| format!("/api/v1/clips/{}.mp4", shot.capture_key)),
            })
        })
        .collect();
    serde_json::json!({ "name": s.name, "lines": lines, "shots": shots })
}

/// `script` with who reads it, how long it runs, and each line's audio.
fn voiced(script: &mut serde_json::Value, edits: &Edits) {
    let Some(voice) = edits.voice.as_ref().and_then(Voicing::describe) else {
        return;
    };
    script["voice"] = serde_json::json!({ "name": voice.name, "listens": edits.listens });
    script["length_ms"] = voice.length_ms.into();
    let Some(lines) = script["lines"].as_array_mut() else {
        return;
    };
    for line in lines {
        let id = line["id"].as_str().unwrap_or_default().to_string();
        if let Some((_, audio)) = voice.lines.iter().find(|(l, _)| *l == id) {
            line["audio"] = audio["audio"].clone();
            line["instruct"] = audio["instruct"].clone();
        }
    }
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

struct Request {
    method: String,
    path: String,
    /// Names lowercased.
    headers: Vec<(String, String)>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// The request line and headers. No route takes a body.
fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    let mut reader = BufReader::new(&*stream);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    Ok(Request {
        method,
        path,
        headers,
    })
}

/// Little-endian f32 samples, as the page sends them.
fn samples(body: &[u8]) -> Vec<f32> {
    body.chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn respond(
    stream: &mut TcpStream,
    status: &str,
    kind: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}
