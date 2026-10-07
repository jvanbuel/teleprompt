//! The API, version 1, over HTTP, and the page.

use super::server::{Flag, OpenSession, Server};
use super::*;
use teleprompt_core::{Progress, Reporter};

/// Where the command listens, for `--format json`: the API's origin and
/// the path its version is served under.
pub fn listening_event(addr: SocketAddr) -> serde_json::Value {
    serde_json::json!({ "event": "listening", "url": format!("http://{addr}"), "api": "/api/v1" })
}

/// The prompters' typeface (`apps/fonts`), Latin, for the page.
const FONT: &[u8] = include_bytes!("page/font.woff2");
const FONT_PATH: &str = "/fonts/atkinson-hyperlegible-next.woff2";
/// The app icon, `apps/icons/teleprompt.svg`, as the pages' tab icon.
const ICON: &[u8] = include_bytes!("page/icon.svg");
const ICON_PATH: &str = "/icon.svg";

/// The page, as one document: its markup, with its style and its script
/// put in where it says. The script is in parts, one per thing the page
/// does, run in this order as one script.
static PAGE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    const SCRIPT: [&str; 6] = [
        include_str!("page/state.js"),
        include_str!("page/takes.js"),
        include_str!("page/voice.js"),
        include_str!("page/edit.js"),
        include_str!("page/shell.js"),
        include_str!("page/main.js"),
    ];
    include_str!("page/index.html")
        .replacen("/* prompt.css */\n", include_str!("page/prompt.css"), 1)
        .replacen("// prompt.js\n", &SCRIPT.concat(), 1)
});

/// Serves `serve` on `listener` until the process ends: the page, and
/// the [`Session`] as API version 1 (`docs/design.md#prompter-api-version-1`):
/// `GET /api/v1/script`, `GET /api/v1/clips/<key>.mp4`, and the session as
/// a WebSocket at `GET /api/v1/session`.
pub fn prompt_on<R: Recognizer + Send + 'static>(
    registry: Registry,
    listener: TcpListener,
    prompt: Prompt,
    recognizer: R,
) -> std::io::Result<()> {
    prompt_watching(registry, listener, prompt, recognizer, None)
}

/// [`prompt_on`] for a script file: placing the shots again when `edits`
/// reloads them, asked each time the script is fetched, which a client
/// does after an edit, and keeping what a take said when asked.
pub fn prompt_watching<R: Recognizer + Send + 'static>(
    registry: Registry,
    listener: TcpListener,
    prompt: Prompt,
    recognizer: R,
    edits: Option<Edits>,
) -> std::io::Result<()> {
    let server = Server::new(registry, None, None);
    *server.session() = Some(Session::new(prompt, Box::new(recognizer) as Hearing)?);
    *write(&server.edits) = edits.map(Arc::new);
    serve_on(listener, server)
}

pub(super) fn serve_on(listener: TcpListener, server: Arc<Server>) -> std::io::Result<()> {
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
        .route("/", get(|| async { Html(PAGE.as_str()) }))
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
    job: impl FnOnce(&mut (dyn FnMut(serde_json::Value) + Send)) -> serde_json::Value + Send + 'static,
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
async fn uses_route(State(server): Shared) -> Response {
    let uses = block_in_place(|| server.setup().uses());
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
    let tools = match server.setup().resolve(&uses) {
        Ok(tools) => tools,
        Err(why) => return failure(StatusCode::BAD_REQUEST, None, vec![why.to_string()]),
    };
    let shared = server.clone();
    streamed(&server, move |say| {
        let setup = shared.setup();
        match setup.install(&tools, &Say(Mutex::new(say))) {
            Ok(_) => serde_json::json!({ "event": "installed", "uses": uses }),
            Err(why) => serde_json::json!({ "event": "failed", "errors": [why.to_string()] }),
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

fn json(value: serde_json::Value) -> Response {
    ([(CONTENT_TYPE, "application/json")], value.to_string()).into_response()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

/// A job's progress, said to the page as the events an app reads.
pub(super) struct Say<'a>(pub(super) Mutex<&'a mut (dyn FnMut(serde_json::Value) + Send)>);

impl Reporter for Say<'_> {
    fn progress(&self, progress: Progress) {
        let mut event = serde_json::json!({ "event": "progress" });
        if let (Some(e), Ok(serde_json::Value::Object(f))) =
            (event.as_object_mut(), serde_json::to_value(progress))
        {
            e.extend(f);
        }
        (self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))(event);
    }
}
