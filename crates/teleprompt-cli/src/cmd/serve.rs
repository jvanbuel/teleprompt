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
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockWriteGuard};

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

/// The recognizer a prompter hears its reader with, chosen when a script
/// is opened.
type Hearing = Box<dyn Recognizer + Send>;

/// What `serve` was started with, for opening a script from the page: the
/// speech model named, and the locale.
pub struct Opener {
    pub model: Option<PathBuf>,
    pub locale: Option<String>,
}

/// Serves the prompter on loopback: `script`'s, opened now, following the
/// reader with the speech model in `ear` or reading it with its voice; or,
/// without one, the page's welcome, which opens a script and sets
/// teleprompt up.
pub fn run_serve(
    script: Option<&std::path::Path>,
    locale: Option<&str>,
    port: u16,
    ear: Ear<'_>,
    format: Format,
) -> Result<(), PromptError> {
    let opener = Opener {
        model: match &ear {
            Ear::Model(model) => model.map(std::path::Path::to_path_buf),
            Ear::Voice => None,
        },
        locale: locale.map(str::to_string),
    };
    let opened = match script {
        Some(script) => Some(opener.open(script, matches!(ear, Ear::Voice)).map_err(
            |e| match e {
                OpenError::Invalid(errors) => PromptError::Validation(errors),
                OpenError::Unheard(why) | OpenError::Busy(why) => PromptError::Runtime(why),
            },
        )?),
        None => None,
    };
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .map_err(|e| PromptError::Runtime(format!("cannot listen on port {port}: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| PromptError::Runtime(e.to_string()))?;
    let says = if opened.is_some() {
        "open it, click, and read"
    } else {
        "open it and choose a script"
    };
    eprintln!("prompting at http://{addr}/ — {says}");
    if format == Format::Json {
        // An app that launched the command reads where to connect from this.
        println!("{}", listening_event(addr));
        std::io::stdout()
            .flush()
            .map_err(|e| PromptError::Runtime(e.to_string()))?;
    }
    let server = Server::new(opened, Some(opener));
    serve_on(listener, server).map_err(|e| PromptError::Runtime(e.to_string()))
}

/// Why a script could not be opened.
pub enum OpenError {
    /// It does not compile.
    Invalid(Vec<String>),
    /// There is nothing to hear the reader with: the speech model, or the
    /// build with the recognizer.
    Unheard(String),
    /// A take is under way.
    Busy(String),
}

/// A script open in the prompter: its session, and what can be done to it.
struct Opened {
    session: Session<Hearing>,
    edits: Edits,
}

impl Opener {
    /// `script`, compiled and ready to read: by ear, or with `voice` by its
    /// voice.
    fn open(&self, script: &std::path::Path, voice: bool) -> Result<Opened, OpenError> {
        let project =
            Project::for_script(script).map_err(|e| OpenError::Invalid(vec![e.to_string()]))?;
        let locale = self
            .locale
            .clone()
            .unwrap_or_else(|| project.source_locale());
        let prompt = prompt_of(&project, script, &locale).map_err(OpenError::Invalid)?;
        let hearing: Hearing = if voice {
            Box::new(teleprompt_listen::Deaf)
        } else {
            let model = self
                .model
                .clone()
                .or_else(|| crate::cmd::setup::speech_model(None).ok());
            recognizer(model.as_deref()).map_err(|e| match e {
                PromptError::Runtime(why) => OpenError::Unheard(why),
                PromptError::Validation(why) => OpenError::Unheard(why.join("\n")),
            })?
        };
        let session =
            Session::new(prompt, hearing).map_err(|e| OpenError::Invalid(vec![e.to_string()]))?;
        let mut edits = edits_of(&project, script, &locale);
        edits.listens = !voice;
        Ok(Opened { session, edits })
    }
}

impl Opener {
    /// What the page's welcome offers: the project around the working
    /// directory and its scripts, the one open, and whether a reader can
    /// be heard here or only a voice can read.
    fn home(&self, opened: Option<String>) -> serde_json::Value {
        let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let project = Project::discover(&here).ok();
        let mut scripts: Vec<PathBuf> = project
            .as_ref()
            .and_then(|p| std::fs::read_dir(p.root.join("scripts")).ok())
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .collect();
        scripts.sort();
        let scripts: Vec<serde_json::Value> = scripts
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.file_name().map(|n| n.to_string_lossy().into_owned()),
                    "path": p,
                })
            })
            .collect();
        let model = self
            .model
            .clone()
            .or_else(|| crate::cmd::setup::speech_model(None).ok());
        serde_json::json!({
            "project": project.as_ref().and_then(|p| p.root.file_name()).map(|n| n.to_string_lossy()),
            "scripts": scripts,
            "opened": opened,
            "hears": cfg!(feature = "listen") && model.is_some(),
            "can_hear": cfg!(feature = "listen"),
        })
    }
}

#[cfg(feature = "listen")]
fn recognizer(model: Option<&std::path::Path>) -> Result<Hearing, PromptError> {
    let dir = model.ok_or_else(|| {
        PromptError::Runtime(format!(
            "`serve` needs a speech model: `teleprompt setup speech-model` installs \
             one, or download and unpack {MODEL} and pass its directory with --model"
        ))
    })?;
    teleprompt_listen_sherpa::SherpaRecognizer::new(dir)
        .map(|r| Box::new(r) as Hearing)
        .map_err(PromptError::Runtime)
}

/// A build without the recognizer cannot follow anyone; it says how to get
/// one, or to let the voice read.
#[cfg(not(feature = "listen"))]
fn recognizer(_model: Option<&std::path::Path>) -> Result<Hearing, PromptError> {
    Err(PromptError::Runtime(
        "this teleprompt was built without a speech recognizer; \
         rebuild it with `--features listen`, or pass --voice to have the \
         script's voice read it"
            .to_string(),
    ))
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

fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(PoisonError::into_inner)
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
    let args: Vec<std::ffi::OsString> = vec![
        job.name().into(),
        script.into(),
        "--locale".into(),
        locale.into(),
    ];
    let report = teleprompt(root, &args, progress)?;
    Ok(report["output"].as_str().map(PathBuf::from))
}

/// Runs this teleprompt with `args` and `--format json` in `dir`, handing
/// on each progress event it reports on stderr; then its report, or why it
/// failed, in its own words.
fn teleprompt(
    dir: &std::path::Path,
    args: &[std::ffi::OsString],
    progress: &mut dyn FnMut(serde_json::Value),
) -> Result<serde_json::Value, String> {
    let me = std::env::current_exe().map_err(|e| format!("cannot find teleprompt: {e}"))?;
    let mut child = std::process::Command::new(&me)
        .args(["--format", "json"])
        .args(args)
        .current_dir(dir)
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
    Ok(report)
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
    let server = Server::new(None, None);
    *server.session() = Some(Session::new(prompt, Box::new(recognizer) as Hearing)?);
    *write(&server.edits) = edits.map(Arc::new);
    serve_on(listener, server)
}

fn serve_on(listener: TcpListener, server: Arc<Server>) -> std::io::Result<()> {
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
fn router(server: Arc<Server>) -> Router {
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
        .route("/api/v1/script", get(script_route))
        .route("/api/v1/manifest", get(manifest_route))
        .route("/api/v1/voice/{file}", get(voice_route))
        .route("/api/v1/clips/{file}", get(clip_route))
        .route("/api/v1/make", post(make_route))
        .route("/api/v1/session", get(session_route))
        .route("/api/v1/home", get(home_route))
        .route("/api/v1/open", post(open_route))
        .route("/api/v1/setup", get(uses_route).post(install_route))
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

type Shared = State<Arc<Server>>;

/// `GET /api/v1/script`: reloaded first if its file has changed.
async fn script_route(State(server): Shared) -> Response {
    match block_in_place(|| server.script()) {
        Some(script) => json(script),
        None => (StatusCode::NOT_FOUND, "no script is open").into_response(),
    }
}

/// `GET /api/v1/manifest`: the manifest `dub` would publish.
async fn manifest_route(State(server): Shared) -> Response {
    let Some(edits) = server.edits().filter(|e| e.voice.is_some()) else {
        return not_found();
    };
    let voice = edits.voice.as_ref().expect("filtered");
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
async fn voice_route(
    State(server): Shared,
    Path(file): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let edits = server.edits();
    let voice = edits.as_ref().and_then(|e| e.voice.as_ref());
    let (Some(id), Some(voice)) = (file.strip_suffix(".wav"), voice) else {
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
async fn clip_route(State(server): Shared, Path(file): Path<String>) -> Response {
    let clip = file
        .strip_suffix(".mp4")
        .and_then(|key| server.session().as_ref()?.clip(key));
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
async fn make_route(
    State(server): Shared,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(job) = query.get("job").and_then(|j| Job::parse(j)) else {
        return (StatusCode::BAD_REQUEST, "job= is capture or build").into_response();
    };
    let Some(edits) = server.edits().filter(|e| e.make.is_some()) else {
        return not_found();
    };
    streamed(&server, move |say| {
        let make = edits.make.as_ref().expect("filtered");
        match make(job, say) {
            Ok(video) => serde_json::json!({ "event": "made", "job": job.name(), "video": video }),
            Err(why) => {
                serde_json::json!({ "event": "failed", "job": job.name(), "errors": [why] })
            }
        }
    })
}

/// A long job's progress events as a streamed body, one JSON object a
/// line as they come, then what `job` answers last. One job at a time.
fn streamed(
    server: &Arc<Server>,
    job: impl FnOnce(&mut dyn FnMut(serde_json::Value)) -> serde_json::Value + Send + 'static,
) -> Response {
    if server.making.swap(true, Ordering::SeqCst) {
        return (
            StatusCode::CONFLICT,
            "a capture, build or install is already running",
        )
            .into_response();
    }
    let server = server.clone();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::task::spawn_blocking(move || {
        let _making = Flag(&server.making);
        // A page gone mid-job leaves the job to finish.
        let mut say = |event: serde_json::Value| {
            let _ = tx.send(format!("{event}\n"));
        };
        let last = job(&mut say);
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

/// `GET /api/v1/home`: what the page offers when no script is open, or to
/// open another: the project's scripts, and whether a reader can be heard
/// here.
async fn home_route(State(server): Shared) -> Response {
    let Some(opener) = &server.opener else {
        return not_found();
    };
    json(block_in_place(|| opener.home(server.opened_name())))
}

/// `POST /api/v1/open?script=<path>[&voice=1]`: opens a script in place of
/// the one open, read by ear or by its voice.
async fn open_route(
    State(server): Shared,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(opener) = &server.opener else {
        return not_found();
    };
    let Some(script) = query.get("script") else {
        return (StatusCode::BAD_REQUEST, "script= names the script").into_response();
    };
    if server.open.load(Ordering::SeqCst) {
        return failure(
            StatusCode::CONFLICT,
            None,
            vec!["a take is under way".into()],
        );
    }
    let voice = query.get("voice").is_some_and(|v| v == "1");
    match block_in_place(|| opener.open(std::path::Path::new(script), voice)) {
        Ok(opened) => {
            let name = opened.session.script().name;
            *server.session() = Some(opened.session);
            *write(&server.edits) = Some(Arc::new(opened.edits));
            json(serde_json::json!({ "ok": true, "name": name }))
        }
        Err(OpenError::Invalid(errors)) => failure(StatusCode::UNPROCESSABLE_ENTITY, None, errors),
        Err(OpenError::Unheard(why)) => failure(StatusCode::CONFLICT, Some("prompt"), vec![why]),
        Err(OpenError::Busy(why)) => failure(StatusCode::CONFLICT, None, vec![why]),
    }
}

/// `GET /api/v1/setup`: what teleprompt can be set up to do here, as
/// `teleprompt setup --uses` says it.
async fn uses_route() -> Response {
    let uses = block_in_place(|| crate::cmd::setup::Setup::detect().uses());
    json(serde_json::to_value(uses).unwrap_or_default())
}

/// `POST /api/v1/setup?uses=<use>,<use>`: installs what they need, as
/// `teleprompt setup <uses> --run` does, its progress events streamed;
/// then `installed` or `failed`.
async fn install_route(
    State(server): Shared,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let uses: Vec<String> = query
        .get("uses")
        .map(|u| {
            u.split(',')
                .filter(|u| !u.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if uses.is_empty() {
        return (StatusCode::BAD_REQUEST, "uses= names what to set up").into_response();
    }
    if let Err(why) = crate::cmd::setup::resolve(&uses) {
        return failure(StatusCode::BAD_REQUEST, None, vec![why]);
    }
    let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    streamed(&server, move |say| {
        let mut args: Vec<std::ffi::OsString> = vec!["setup".into()];
        args.extend(uses.iter().map(Into::into));
        args.push("--run".into());
        match teleprompt(&dir, &args, say) {
            Ok(_) => serde_json::json!({ "event": "installed", "uses": uses }),
            Err(why) => serde_json::json!({ "event": "failed", "errors": [why] }),
        }
    })
}

/// A refusal the page can act on: why, and what to set up first.
fn failure(status: StatusCode, needs: Option<&str>, errors: Vec<String>) -> Response {
    (
        status,
        [(CONTENT_TYPE, "application/json")],
        serde_json::json!({ "ok": false, "needs": needs, "errors": errors }).to_string(),
    )
        .into_response()
}

/// `GET /api/v1/session`: the session socket, one at a time.
async fn session_route(
    State(server): Shared,
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
struct OpenSession(Arc<Server>);

impl Drop for OpenSession {
    fn drop(&mut self) {
        self.0.open.store(false, Ordering::SeqCst);
    }
}

struct Server {
    /// The open script's session; none until a script is opened.
    session: Mutex<Option<Session<Hearing>>>,
    /// What can be done to the open script's file.
    edits: RwLock<Option<Arc<Edits>>>,
    /// How to open a script from the page; none where only the one given
    /// is served.
    opener: Option<Opener>,
    /// Whether a session socket is open.
    open: AtomicBool,
    /// Whether a capture, build or install is running.
    making: AtomicBool,
}

impl Server {
    fn new(opened: Option<Opened>, opener: Option<Opener>) -> Arc<Self> {
        let (session, edits) = match opened {
            Some(o) => (Some(o.session), Some(Arc::new(o.edits))),
            None => (None, None),
        };
        Arc::new(Self {
            session: Mutex::new(session),
            edits: RwLock::new(edits),
            opener,
            open: AtomicBool::new(false),
            making: AtomicBool::new(false),
        })
    }

    fn session(&self) -> MutexGuard<'_, Option<Session<Hearing>>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn edits(&self) -> Option<Arc<Edits>> {
        self.edits
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn opened_name(&self) -> Option<String> {
        self.session().as_ref().map(|s| s.script().name)
    }

    /// The script as the page draws it, reloaded first if its file has
    /// changed; compiled outside the lock, which the session needs. None
    /// while no script is open.
    fn script(&self) -> Option<serde_json::Value> {
        let edits = self.edits();
        let edited = edits.as_ref().and_then(|e| (e.reload)());
        let mut session = self.session();
        let session = session.as_mut()?;
        if let Some(prompt) = edited {
            session.replace(prompt);
        }
        let mut script = script(session.script());
        if let Some(edits) = &edits {
            voiced(&mut script, edits);
        }
        Some(script)
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
                    let reached = block_in_place(|| {
                        self.session()
                            .as_mut()
                            .map(|s| s.listen(&samples(&audio), rate))
                    });
                    let Some(reached) = reached else {
                        return;
                    };
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
        let edits = self.edits();
        let mut session = self.session();
        let Some(session) = session.as_mut() else {
            return Some(error("no script is open".into()));
        };
        match message["type"].as_str() {
            Some("start") if !edits.as_ref().is_none_or(|e| e.listens) => Some(error(
                "this prompter reads the script with its voice; it does not listen".into(),
            )),
            Some("start") => {
                let from = message["from"].as_u64().unwrap_or(0) as usize;
                *rate = message["rate"]
                    .as_u64()
                    .and_then(|r| u32::try_from(r).ok())
                    .filter(|&r| r > 0)
                    .unwrap_or(LISTEN_RATE);
                let reached = session.start(from);
                *at = Some(reached.at);
                Some(reached_json(&reached))
            }
            Some("stop") => Some(match session.stop() {
                Ok(saved) => serde_json::json!({ "type": "stopped", "saved": saved }),
                Err(e) => error(e.to_string()),
            }),
            Some("discard") => {
                session.discard();
                Some(serde_json::json!({ "type": "discarded" }))
            }
            Some("undo") => Some(match session.undo() {
                Ok(lines) => serde_json::json!({ "type": "undone", "lines": lines }),
                Err(e) => error(e.to_string()),
            }),
            Some("keep_said") => {
                let line = message["line"].as_str().unwrap_or_default();
                Some(match &edits {
                    Some(edits) => match (edits.keep_said)(line) {
                        Ok(()) => serde_json::json!({ "type": "kept_said", "line": line }),
                        Err(e) => error(e),
                    },
                    None => error("this prompter has no script file to reword".into()),
                })
            }
            Some("undo_edit") => Some(match &edits {
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
                Some(match &edits {
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
    /// The script to open; without one, the page opens on its welcome,
    /// which lists the project's scripts and sets teleprompt up
    pub script: Option<std::path::PathBuf>,
    /// The locale to compile for; the project's `locales.source` if not given
    #[arg(long, value_parser = crate::cli::language_tag)]
    pub locale: Option<String>,
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
    if let Some(script) = &args.script {
        // A script outside any project is said as every command says it.
        crate::cli::project_for(script)?;
    }
    let model = (!args.voice).then(|| {
        args.model
            .or_else(|| crate::cmd::setup::speech_model(None).ok())
    });
    run_serve(
        args.script.as_deref(),
        args.locale.as_deref(),
        args.port,
        match &model {
            Some(model) => Ear::Model(model.as_deref()),
            None => Ear::Voice,
        },
        format,
    )?;
    Ok(Outcome::Ok)
}
