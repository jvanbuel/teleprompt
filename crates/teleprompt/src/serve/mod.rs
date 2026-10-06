//! `teleprompt serve <script>`: a prompter that follows the reader's voice.
//!
//! The prompter itself is `crate::serve::prompter`; this is its API, version 1
//! (`routes`), and the page that drives it: HTTP for the script and clips, and a
//! WebSocket for the session, served with axum on loopback, for one reader.
//! The session, compiling and voicing are blocking work, done in place on
//! the runtime's worker.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockWriteGuard};

use crate::serve::prompter::{Position, Prompt, Reached, ScriptView, Session, LISTEN_RATE};
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
use tokio::task::block_in_place;

use crate::project::{Project, Script};
use crate::registry::Registry;
use crate::serve::voicing::Voicing;
use crate::Failure;
use teleprompt_script::edit::Edit;

mod loopback;
pub mod prompter;
mod routes;
mod server;
pub mod voicing;

use routes::serve_on;
pub use routes::{listening_event, prompt_on, prompt_watching};
use server::Server;

/// The speech model `serve` is tested with: sherpa-onnx's streaming
/// English zipformer. The smaller 20M model misses the first words of a
/// stream. teleprompt downloads nothing, so the error naming it says where
/// to get it.
pub const MODEL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-2023-06-26.tar.bz2";

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
    pub registry: Registry,
    pub model: Option<PathBuf>,
    pub locale: Option<String>,
}

/// Serves the prompter on loopback: `script`'s, opened now, following the
/// reader with the speech model in `ear` or reading it with its voice; or,
/// without one, the page's welcome, which opens a script and sets
/// teleprompt up. With `json`, where to connect is also said on stdout, as
/// an event an app that launched it reads.
pub fn run_serve(
    registry: Registry,
    script: Option<&std::path::Path>,
    locale: Option<&str>,
    port: u16,
    ear: Ear<'_>,
    json: bool,
) -> Result<(), Failure> {
    let opener = Opener {
        registry,
        model: match &ear {
            Ear::Model(model) => model.map(std::path::Path::to_path_buf),
            Ear::Voice => None,
        },
        locale: locale.map(str::to_string),
    };
    let opened = match script {
        Some(script) => Some(opener.open(script, matches!(ear, Ear::Voice)).map_err(
            |e| match e {
                OpenError::Invalid(errors) => Failure::Validation(errors),
                OpenError::Unheard(why) | OpenError::Busy(why) => Failure::Runtime(why),
            },
        )?),
        None => None,
    };
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .map_err(|e| Failure::Runtime(format!("cannot listen on port {port}: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| Failure::Runtime(e.to_string()))?;
    let says = if opened.is_some() {
        "open it, click, and read"
    } else {
        "open it and choose a script"
    };
    eprintln!("prompting at http://{addr}/ — {says}");
    if json {
        println!("{}", listening_event(addr));
        std::io::stdout()
            .flush()
            .map_err(|e| Failure::Runtime(e.to_string()))?;
    }
    let server = Server::new(registry, opened, Some(opener));
    serve_on(listener, server).map_err(|e| Failure::Runtime(e.to_string()))
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
        let project = Project::for_script(script, self.registry)
            .map_err(|e| OpenError::Invalid(vec![e.to_string()]))?;
        let locale = self
            .locale
            .clone()
            .unwrap_or_else(|| project.source_locale());
        let opened = project.script(script, locale);
        let prompt = opened.prompt().map_err(OpenError::Invalid)?;
        let hearing: Hearing = if voice {
            Box::new(teleprompt_listen::Deaf)
        } else {
            let model = self
                .model
                .clone()
                .or_else(|| crate::setup::speech_model(None).ok());
            recognizer(model.as_deref()).map_err(|e| match e {
                Failure::Runtime(why) => OpenError::Unheard(why),
                Failure::Validation(why) => OpenError::Unheard(why.join("\n")),
            })?
        };
        let session =
            Session::new(prompt, hearing).map_err(|e| OpenError::Invalid(vec![e.to_string()]))?;
        let mut edits = opened.edits();
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
        let project = Project::discover(&here, self.registry).ok();
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
            .or_else(|| crate::setup::speech_model(None).ok());
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
fn recognizer(model: Option<&std::path::Path>) -> Result<Hearing, Failure> {
    let dir = model.ok_or_else(|| {
        Failure::Runtime(format!(
            "`serve` needs a speech model: `teleprompt setup speech-model` installs \
             one, or download and unpack {MODEL} and pass its directory with --model"
        ))
    })?;
    teleprompt_listen::sherpa::SherpaRecognizer::new(dir)
        .map(|r| Box::new(r) as Hearing)
        .map_err(Failure::Runtime)
}

/// A build without the recognizer cannot follow anyone; it says how to get
/// one, or to let the voice read.
#[cfg(not(feature = "listen"))]
fn recognizer(_model: Option<&std::path::Path>) -> Result<Hearing, Failure> {
    Err(Failure::Runtime(
        "this teleprompt was built without a speech recognizer; \
         rebuild it with `--features listen`, or pass --voice to have the \
         script's voice read it"
            .to_string(),
    ))
}

impl Script {
    /// What the prompter shows of the script, as it now reads.
    fn prompt(&self) -> Result<Prompt, Vec<String>> {
        let project = self.project();
        let compiled = self.compile()?.output;
        Ok(Prompt {
            name: self
                .path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            shots: crate::serve::prompter::shot_cues(&compiled),
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

    /// The script again whenever its file has changed since last asked,
    /// for one edited while it is being read: a shot moved or stretched, a
    /// line reworded. `None` when it has not changed, or does not compile.
    pub fn reload_on_edit(&self) -> Reload {
        let seen = Mutex::new(crate::project::fingerprint(self.path()));
        let script = self.clone();
        Box::new(move || {
            let now = crate::project::fingerprint(script.path());
            let mut seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
            if now == *seen {
                return None;
            }
            *seen = now;
            script.prompt().ok()
        })
    }
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

impl Script {
    /// The script's edits: reloaded when changed, a line reworded as
    /// `teleprompt edit <script> said <line>` does, each edit kept to undo,
    /// and captured or built as the commands do.
    pub fn edits(&self) -> Edits {
        let keeper = self.clone();
        let editor = self.clone();
        // The script before and after each edit, latest last.
        let history = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let (kept, undone) = (history.clone(), self.path().to_path_buf());
        let maker = self.clone();
        Edits {
            reload: self.reload_on_edit(),
            keep_said: Box::new(move |line| {
                keeper
                    .keep_said(line)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }),
            edit: Box::new(move |edit| {
                let edited = editor.path();
                let before = std::fs::read_to_string(edited).map_err(|e| e.to_string())?;
                editor.edit(edit).map_err(|e| e.to_string())?;
                let after = std::fs::read_to_string(edited).map_err(|e| e.to_string())?;
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
                make(
                    &maker.project().root,
                    maker.path(),
                    maker.locale(),
                    job,
                    progress,
                )
            })),
            voice: Some(Voicing::new(self.clone())),
            listens: true,
        }
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
