//! `teleprompt serve <script>`: a prompter that follows the reader's voice.
//!
//! The prompter itself is `teleprompt-prompter`; this is its API, version
//! 1, and the page that drives it: HTTP for the script and clips, and a
//! WebSocket for the session, served with axum on loopback, for one reader.
//! The session, compiling and voicing are blocking work, done in place on
//! the runtime's worker.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, HOST, ORIGIN, UPGRADE};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use teleprompt_listen::Recognizer;
use teleprompt_prompter::{Position, Prompt, Reached, Script, Session, LISTEN_RATE};
use tokio::task::block_in_place;

use crate::cmd::voicing::Voicing;
use crate::output::{Format, Outcome};
use crate::project::Project;
use teleprompt_core::edit::Edit;

/// The speech model `serve` is tested with: sherpa-onnx's streaming
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
pub fn run_serve(
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
            "`serve` needs a speech model: `teleprompt setup speech-model` installs \
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
    let (compiled, _) = project.compile(script, locale)?;
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

/// Puts back what the last edit changed, or says why not.
pub type UndoEdit = Box<dyn Fn() -> Result<(), String> + Send + Sync>;

/// Runs a job on the script, handing on each progress event it reports;
/// then the video, for a build.
pub type Make = Box<
    dyn Fn(Job, &mut dyn FnMut(serde_json::Value)) -> Result<Option<PathBuf>, String> + Send + Sync,
>;

/// What a prompter can have made of its script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// `teleprompt capture`: the shots not yet captured, or changed since.
    Capture,
    /// `teleprompt build`: captured, then rendered.
    Build,
}

impl Job {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "capture" => Some(Self::Capture),
            "build" => Some(Self::Build),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Build => "build",
        }
    }
}

/// What a prompter reading a script file can do to it.
pub struct Edits {
    pub reload: Reload,
    pub keep_said: KeepSaid,
    pub edit: EditScript,
    pub undo: UndoEdit,
    /// None where nothing can be captured or built from it.
    pub make: Option<Make>,
    /// How its lines sound when its voice reads them; none for a script
    /// that is not a project's file.
    pub voice: Option<Voicing>,
    /// Whether it follows a reader by ear; if not, its voice reads.
    pub listens: bool,
}

/// `script`'s edits: reloaded when changed, a line reworded as
/// `teleprompt edit <script> said <line>` does, each edit kept to undo,
/// and captured or built as the commands do.
pub fn edits_of(project: &Project, script: &std::path::Path, locale: &str) -> Edits {
    let (keeper, path) = (project.clone(), script.to_path_buf());
    let (editor, edited) = (project.clone(), script.to_path_buf());
    // The script before and after each edit, latest last.
    let history = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let (kept, undone) = (history.clone(), script.to_path_buf());
    let (root, made, locale_of) = (
        project.root.clone(),
        script.to_path_buf(),
        locale.to_string(),
    );
    Edits {
        reload: reload_on_edit(project, script, locale),
        keep_said: Box::new(move |line| {
            crate::cmd::edit::run_said(&keeper, &path, line)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }),
        edit: Box::new(move |edit| {
            let before = std::fs::read_to_string(&edited).map_err(|e| e.to_string())?;
            crate::cmd::edit::run_edit(&editor, &edited, edit).map_err(|e| e.to_string())?;
            let after = std::fs::read_to_string(&edited).map_err(|e| e.to_string())?;
            if after != before {
                lock(&kept).push((before, after));
            }
            Ok(())
        }),
        undo: Box::new(move || {
            let (before, after) = lock(&history)
                .pop()
                .ok_or_else(|| "nothing to undo".to_string())?;
            let now = std::fs::read_to_string(&undone).map_err(|e| e.to_string())?;
            if now != after {
                lock(&history).clear();
                return Err("the script has changed since: undo it in your editor".into());
            }
            std::fs::write(&undone, before).map_err(|e| e.to_string())
        }),
        make: Some(Box::new(move |job, progress| {
            make(&root, &made, &locale_of, job, progress)
        })),
        voice: Some(Voicing::new(project, script, locale)),
        listens: true,
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `job` as the command would be run, from `root`, handing on each
/// progress event it reports on stderr; then the video, for a build.
fn make(
    root: &std::path::Path,
    script: &std::path::Path,
    locale: &str,
    job: Job,
    progress: &mut dyn FnMut(serde_json::Value),
) -> Result<Option<PathBuf>, String> {
    let me = std::env::current_exe().map_err(|e| format!("cannot find teleprompt: {e}"))?;
    let mut child = std::process::Command::new(&me)
        .args(["--format", "json", job.name()])
        .arg(script)
        .args(["--locale", locale])
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", me.display()))?;
    let stdout = child.stdout.take().expect("piped");
    let report = std::thread::spawn(move || std::io::read_to_string(stdout).unwrap_or_default());
    let mut said = String::new();
    for line in BufReader::new(child.stderr.take().expect("piped")).lines() {
        let line = line.unwrap_or_default();
        match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(event) if event["event"] == "progress" => progress(event),
            _ => {
                said.push_str(&line);
                said.push('\n');
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let report: serde_json::Value =
        serde_json::from_str(&report.join().unwrap_or_default()).unwrap_or_default();
    if !status.success() {
        let errors: Vec<&str> = report["errors"]
            .as_array()
            .map(|e| e.iter().filter_map(serde_json::Value::as_str).collect())
            .unwrap_or_default();
        return Err(if errors.is_empty() {
            said.trim().to_string()
        } else {
            errors.join("\n")
        });
    }
    Ok(report["output"].as_str().map(PathBuf::from))
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

/// Serves `serve` on `listener` until the process ends: the page, and
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
        making: AtomicBool::new(false),
        edits,
    });
    listener.set_nonblocking(true)?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener)?;
            axum::serve(listener, router(server)).await
        })
}

/// The page and API version 1, behind the loopback guard.
fn router<R: Recognizer + Send + 'static>(server: Arc<Server<R>>) -> Router {
    Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/favicon.ico", get(|| async { StatusCode::NO_CONTENT }))
        .route(
            FONT_PATH,
            get(|| async { ([(CONTENT_TYPE, "font/woff2")], FONT) }),
        )
        .route(
            ICON_PATH,
            get(|| async { ([(CONTENT_TYPE, "image/svg+xml")], ICON) }),
        )
        .route("/api/v1/script", get(script_route::<R>))
        .route("/api/v1/manifest", get(manifest_route::<R>))
        .route("/api/v1/voice/{file}", get(voice_route::<R>))
        .route("/api/v1/clips/{file}", get(clip_route::<R>))
        .route("/api/v1/make", post(make_route::<R>))
        .route("/api/v1/session", get(session_route::<R>))
        .fallback(|| async { not_found() })
        .layer(middleware::from_fn(guard))
        .with_state(server)
}

/// Refuses a request that names another host or comes from another
/// site's page (`crate::loopback`), and lets no answer be cached.
async fn guard(request: Request, next: Next) -> Response {
    let refused = {
        let header = |name| request.headers().get(name).and_then(|v| v.to_str().ok());
        crate::loopback::refused(header(HOST), header(ORIGIN))
    };
    if let Some(why) = refused {
        return (StatusCode::FORBIDDEN, why).into_response();
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

type Shared<R> = State<Arc<Server<R>>>;

/// `GET /api/v1/script`: reloaded first if its file has changed.
async fn script_route<R: Recognizer + Send + 'static>(State(server): Shared<R>) -> Response {
    block_in_place(|| json(server.script()))
}

/// `GET /api/v1/manifest`: the manifest `dub` would publish.
async fn manifest_route<R: Recognizer + Send + 'static>(State(server): Shared<R>) -> Response {
    let Some(voice) = server.voice() else {
        return not_found();
    };
    match block_in_place(|| voice.manifest()) {
        Ok(manifest) => json(serde_json::to_value(manifest).unwrap_or_default()),
        Err(errors) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(CONTENT_TYPE, "application/json")],
            serde_json::json!({ "ok": false, "errors": errors }).to_string(),
        )
            .into_response(),
    }
}

/// `GET /api/v1/voice/<line>.wav`, `?fresh=1` made anew, `?fit=1` as the
/// manifest publishes it.
async fn voice_route<R: Recognizer + Send + 'static>(
    State(server): Shared<R>,
    Path(file): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let (Some(id), Some(voice)) = (file.strip_suffix(".wav"), server.voice()) else {
        return not_found();
    };
    let asked = |flag: &str| query.get(flag).is_some_and(|v| v == "1");
    match block_in_place(|| voice.audio(id, asked("fresh"), asked("fit"))) {
        Ok(Some(wav)) => ([(CONTENT_TYPE, "audio/wav")], wav).into_response(),
        Ok(None) => not_found(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// `GET /api/v1/clips/<key>.mp4`: a shot's captured clip.
async fn clip_route<R: Recognizer + Send + 'static>(
    State(server): Shared<R>,
    Path(file): Path<String>,
) -> Response {
    let clip = file
        .strip_suffix(".mp4")
        .and_then(|key| server.session().clip(key));
    let Some(clip) = clip else {
        return not_found();
    };
    match block_in_place(|| std::fs::read(clip)) {
        Ok(bytes) => ([(CONTENT_TYPE, "video/mp4")], bytes).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// `POST /api/v1/make?job=capture|build`: the job's progress events, one
/// JSON object a line as they come, then `made` or `failed`.
async fn make_route<R: Recognizer + Send + 'static>(
    State(server): Shared<R>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(job) = query.get("job").and_then(|j| Job::parse(j)) else {
        return (StatusCode::BAD_REQUEST, "job= is capture or build").into_response();
    };
    if server
        .edits
        .as_ref()
        .and_then(|e| e.make.as_ref())
        .is_none()
    {
        return not_found();
    }
    if server.making.swap(true, Ordering::SeqCst) {
        return (
            StatusCode::CONFLICT,
            "a capture or build is already running",
        )
            .into_response();
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::task::spawn_blocking(move || {
        let _making = Flag(&server.making);
        let make = server.edits.as_ref().and_then(|e| e.make.as_ref());
        // A page gone mid-job leaves the job to finish.
        let mut say = |event: serde_json::Value| {
            let _ = tx.send(format!("{event}\n"));
        };
        let last = match make.map(|make| make(job, &mut say)) {
            Some(Ok(video)) => {
                serde_json::json!({ "event": "made", "job": job.name(), "video": video })
            }
            Some(Err(why)) => {
                serde_json::json!({ "event": "failed", "job": job.name(), "errors": [why] })
            }
            None => return,
        };
        say(last);
    });
    let lines = futures_util::stream::unfold(rx, |mut rx| async move {
        let line = rx.recv().await?;
        Some((Ok::<_, std::convert::Infallible>(line), rx))
    });
    (
        [(CONTENT_TYPE, "application/x-ndjson")],
        Body::from_stream(lines),
    )
        .into_response()
}

/// `GET /api/v1/session`: the session socket, one at a time.
async fn session_route<R: Recognizer + Send + 'static>(
    State(server): Shared<R>,
    upgrade: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    let Ok(upgrade) = upgrade else {
        return (
            StatusCode::UPGRADE_REQUIRED,
            [(UPGRADE, "websocket")],
            "the session is a WebSocket",
        )
            .into_response();
    };
    if server.open.swap(true, Ordering::SeqCst) {
        return (StatusCode::CONFLICT, "a session is already open").into_response();
    }
    // Let go when the session ends, a panic in it or an upgrade that
    // never completes included.
    let open = OpenSession(server.clone());
    upgrade.on_upgrade(move |socket| async move {
        let _open = open;
        server.run_session(socket).await;
    })
}

/// Clears a flag when dropped.
struct Flag<'a>(&'a AtomicBool);

impl Drop for Flag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Marks the session closed when dropped.
struct OpenSession<R>(Arc<Server<R>>);

impl<R> Drop for OpenSession<R> {
    fn drop(&mut self) {
        self.0.open.store(false, Ordering::SeqCst);
    }
}

struct Server<R> {
    session: Mutex<Session<R>>,
    /// Whether a session socket is open.
    open: AtomicBool,
    /// Whether a capture or build is running.
    making: AtomicBool,
    edits: Option<Edits>,
}

impl<R: Recognizer> Server<R> {
    fn session(&self) -> MutexGuard<'_, Session<R>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn voice(&self) -> Option<&Voicing> {
        self.edits.as_ref().and_then(|e| e.voice.as_ref())
    }

    /// The script as the page draws it, reloaded first if its file has
    /// changed; compiled outside the lock, which the session needs.
    fn script(&self) -> serde_json::Value {
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
        script
    }

    async fn run_session(&self, mut socket: WebSocket) {
        let mut rate = LISTEN_RATE;
        let mut at = None;
        while let Some(Ok(message)) = socket.recv().await {
            let answer = match message {
                Message::Text(text) => {
                    block_in_place(|| self.command(text.as_str(), &mut rate, &mut at))
                }
                Message::Binary(audio) => {
                    let reached = block_in_place(|| self.session().listen(&samples(&audio), rate));
                    let news = at != Some(reached.at) || !reached.play.is_empty();
                    at = Some(reached.at);
                    news.then(|| reached_json(&reached))
                }
                Message::Close(_) => return,
                _ => None,
            };
            if let Some(answer) = answer {
                if socket
                    .send(Message::Text(answer.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
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
            Some("undo_edit") => Some(match &self.edits {
                Some(edits) => match (edits.undo)() {
                    Ok(()) => serde_json::json!({ "type": "edit_undone" }),
                    Err(e) => error(e),
                },
                None => error("this prompter has no script file to edit".into()),
            }),
            Some("reword" | "instruct" | "cue" | "hold" | "move" | "stretch") => {
                let (edit, edited) = match edit_of(&message) {
                    Ok(edit) => edit,
                    Err(e) => return Some(error(e)),
                };
                Some(match &self.edits {
                    Some(edits) => match (edits.edit)(&edit) {
                        Ok(()) => edited,
                        Err(e) => error(e),
                    },
                    None => error("this prompter has no script file to edit".into()),
                })
            }
            _ => Some(error(format!("not a message this server knows: {text}"))),
        }
    }
}

/// The edit a message asks for, and the answer once it is made: naming
/// the line it changed, or the block it moved.
fn edit_of(message: &serde_json::Value) -> Result<(Edit, serde_json::Value), String> {
    let text = |key: &str| message[key].as_str().map(str::to_string);
    let line = || text("line").ok_or("which line? `line` names it");
    let block = || text("block").ok_or("which block? `block` names it");
    let word = || message["word"].as_u64().map(|w| w as usize);
    let edit = match message["type"].as_str().unwrap_or_default() {
        "reword" => Edit::Reword {
            line: line()?.into(),
            text: text("text").ok_or("a reword needs its text")?,
        },
        "instruct" => Edit::Instruct {
            line: line()?.into(),
            text: text("text").filter(|t| !t.trim().is_empty()),
        },
        "cue" => Edit::Cue {
            block: block()?.into(),
            word: word().ok_or("a cue needs its `word`")?,
        },
        "hold" => Edit::Hold {
            block: block()?.into(),
        },
        "move" => Edit::Move {
            block: block()?.into(),
            after: text("after")
                .ok_or("a move needs the line it goes `after`")?
                .into(),
            word: word(),
        },
        _ => Edit::Stretch {
            block: block()?.into(),
            by: message["by"]
                .as_f64()
                .filter(|by| *by > 0.0 && by.is_finite())
                .ok_or("a stretch needs `by`, more than 0")?,
        },
    };
    let edited = match &edit {
        Edit::Reword { line, .. } | Edit::Instruct { line, .. } => {
            serde_json::json!({ "type": "edited", "line": line })
        }
        Edit::Cue { block, .. }
        | Edit::Hold { block }
        | Edit::Move { block, .. }
        | Edit::Stretch { block, .. } => serde_json::json!({ "type": "edited", "block": block }),
    };
    Ok((edit, edited))
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
    script["timeline"] = voice.timeline;
    script["error"] = voice.error.into();
    let Some(lines) = script["lines"].as_array_mut() else {
        return;
    };
    for line in lines {
        let id = line["id"].as_str().unwrap_or_default().to_string();
        if let Some((_, audio)) = voice.lines.iter().find(|(l, _)| *l == id) {
            line["audio"] = audio["audio"].clone();
            line["instruct"] = audio["instruct"].clone();
            line["speaker"] = audio["speaker"].clone();
        }
    }
}

fn json(value: serde_json::Value) -> Response {
    ([(CONTENT_TYPE, "application/json")], value.to_string()).into_response()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

/// Little-endian f32 samples, as the page sends them.
fn samples(body: &[u8]) -> Vec<f32> {
    body.chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// `serve`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub script: crate::cli::ScriptArgs,
    /// Port to listen on; 0 picks a free one
    #[arg(long, default_value_t = 7879)]
    pub port: u16,
    /// Directory of an unpacked sherpa-onnx streaming zipformer model;
    /// defaults to the one `teleprompt setup speech-model` installed
    #[arg(long)]
    pub model: Option<PathBuf>,
    /// Read the script with its voice instead of following yours:
    /// needs no speech model, in any build
    #[arg(long, conflicts_with = "model")]
    pub voice: bool,
}

/// `serve`, following the reader by ear with the speech model named or
/// installed, or with `--voice`, reading the script with its voice.
pub fn run(args: Args, format: Format) -> crate::cli::Run {
    let project = args.script.project()?;
    let model = (!args.voice).then(|| {
        args.model
            .or_else(|| crate::cmd::setup::speech_model(None).ok())
    });
    run_serve(
        &project,
        &args.script.script,
        &args.script.locale(&project),
        args.port,
        match &model {
            Some(model) => Ear::Model(model.as_deref()),
            None => Ear::Voice,
        },
        format,
    )?;
    Ok(Outcome::Ok)
}
