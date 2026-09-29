//! `teleprompt prompt`'s API, v1, driven over real sockets with a
//! recognizer that hears what the test says. What the prompter does is
//! tested in `teleprompt-prompter`; these pin the routes, the socket's
//! messages and their JSON.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use teleprompt_cli::cmd::prompt::prompt_on;
use teleprompt_core::{Hash, LineId};
use teleprompt_listen::{Heard, Position, Recognizer};
use teleprompt_prompter::{Prompt, ShotCue};

/// Hears what the test says, one hypothesis per chunk, and counts the
/// samples it was given.
struct Scripted(VecDeque<&'static str>, Arc<AtomicUsize>);

impl Recognizer for Scripted {
    fn listen(&mut self, samples: &[f32]) -> Heard {
        self.1.fetch_add(samples.len(), Ordering::SeqCst);
        Heard {
            text: self.0.pop_front().unwrap_or_default().to_string(),
            is_final: false,
        }
    }

    fn reset(&mut self) {}
}

const LINES: &[&str] = &[
    "Welcome to Acme. Let me show you around.",
    "Deployment is one command.",
];

const IDS: &[&str] = &["welcome", "deploy"];

/// A shot before the first line, one three words in, and one after the
/// first line; only the second has been captured.
fn shots() -> Vec<ShotCue> {
    let cue = |shot: &str, line, word| ShotCue {
        shot: shot.into(),
        capture_key: Hash::of(shot.as_bytes()),
        at: Position { line, word },
    };
    vec![
        cue("intro#0", 0, 0),
        cue("welcome-a#0", 0, 3),
        cue("welcome-b#0", 1, 0),
    ]
}

const CLIP: &[u8] = b"the bytes of a captured clip";

/// A prompter served on a port the OS picks, hearing `heard` in turn. The
/// directory holds its clips, and lives as long as the test keeps it.
fn prompting(heard: &[&'static str]) -> (SocketAddr, teleprompt_testkit::TestDir) {
    let (addr, dir, _) = prompting_counted(heard);
    (addr, dir)
}

/// [`prompting`], and a count of the samples the recognizer was given.
fn prompting_counted(
    heard: &[&'static str],
) -> (SocketAddr, teleprompt_testkit::TestDir, Arc<AtomicUsize>) {
    let clips = teleprompt_testkit::test_dir("prompt-clips");
    let captured = Hash::of(b"welcome-a#0");
    std::fs::write(clips.join(format!("{captured}.mp4")), CLIP).unwrap();
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    let addr = listener.local_addr().unwrap();
    let prompt = Prompt {
        name: "tour.md".into(),
        lines: LINES.iter().map(|l| l.to_string()).collect(),
        ids: IDS.iter().map(|l| LineId::from(*l)).collect(),
        shots: shots(),
        clips: clips.to_path_buf(),
        takes: clips.join("takes"),
    };
    let count = Arc::new(AtomicUsize::new(0));
    let recognizer = Scripted(heard.iter().copied().collect(), count.clone());
    std::thread::spawn(move || prompt_on(listener, prompt, recognizer));
    (addr, clips, count)
}

/// One request, and the response body.
fn request(addr: SocketAddr, method: &str, path: &str, body: &[u8]) -> String {
    let mut s = TcpStream::connect(addr).unwrap();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .unwrap();
    s.write_all(body).unwrap();
    let mut response = String::new();
    s.read_to_string(&mut response).unwrap();
    response.split_once("\r\n\r\n").unwrap().1.to_string()
}

/// A GET, and the whole response: status line, headers and body.
fn get(addr: SocketAddr, path: &str) -> Vec<u8> {
    let mut s = TcpStream::connect(addr).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = Vec::new();
    s.read_to_end(&mut response).unwrap();
    response
}

fn json_at(addr: SocketAddr, path: &str) -> serde_json::Value {
    let body = request(addr, "GET", path, &[]);
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("{path}: {e}: {body}"))
}

type Socket = tungstenite::WebSocket<TcpStream>;

/// The session socket, opened as the page opens it.
fn session(addr: SocketAddr) -> Socket {
    let stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let url = format!("ws://{addr}/api/v1/session");
    tungstenite::client(url.as_str(), stream).unwrap().0
}

fn send(ws: &mut Socket, message: serde_json::Value) {
    ws.send(tungstenite::Message::text(message.to_string()))
        .unwrap();
}

/// Sends `n` samples of silence as one binary message.
fn send_audio(ws: &mut Socket, n: usize) {
    ws.send(tungstenite::Message::binary(vec![0u8; n * 4]))
        .unwrap();
}

/// The next JSON message from the server.
fn next(ws: &mut Socket) -> serde_json::Value {
    loop {
        match ws.read().unwrap() {
            tungstenite::Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            tungstenite::Message::Ping(_) | tungstenite::Message::Pong(_) => continue,
            other => panic!("expected JSON, got {other:?}"),
        }
    }
}

/// Starting a take answers where it starts and what plays there.
#[test]
fn starting_a_take_answers_where_the_reader_is() {
    let (addr, _clips) = prompting(&["deployment is"]);
    let mut ws = session(addr);
    send(
        &mut ws,
        serde_json::json!({ "type": "start", "from": 1, "rate": 16000 }),
    );
    assert_eq!(
        next(&mut ws),
        serde_json::json!({ "type": "reached", "line": 1, "word": 0, "play": ["welcome-b#0"] })
    );
    send_audio(&mut ws, 1600);
    assert_eq!(
        next(&mut ws),
        serde_json::json!({ "type": "reached", "line": 1, "word": 2, "play": [] })
    );
}

/// Audio that moves no one says nothing; the page hears only news.
#[test]
fn audio_that_moves_no_one_says_nothing() {
    let (addr, _clips) = prompting(&["", "welcome to"]);
    let mut ws = session(addr);
    send(
        &mut ws,
        serde_json::json!({ "type": "start", "from": 0, "rate": 16000 }),
    );
    next(&mut ws);
    send_audio(&mut ws, 1600);
    send_audio(&mut ws, 1600);
    assert_eq!(
        next(&mut ws),
        serde_json::json!({ "type": "reached", "line": 0, "word": 2, "play": [] }),
        "the first message after the silence"
    );
}

/// The take's audio is at the rate `start` names; the recognizer hears it
/// at its own.
#[test]
fn audio_is_heard_at_the_recognizers_rate() {
    let (addr, _dir, heard) = prompting_counted(&[]);
    let mut ws = session(addr);
    send(
        &mut ws,
        serde_json::json!({ "type": "start", "from": 0, "rate": 48000 }),
    );
    next(&mut ws);
    send_audio(&mut ws, 4_800);
    send_audio(&mut ws, 4_800);
    send(&mut ws, serde_json::json!({ "type": "stop" }));
    next(&mut ws);
    let n = heard.load(Ordering::SeqCst);
    assert!((3_100..=3_200).contains(&n), "{n} samples at 16 kHz");
}

/// Stopping answers the ids of the lines kept.
#[test]
fn stopping_answers_the_lines_kept() {
    let (addr, _dir) = prompting(&[]);
    let mut ws = session(addr);
    send(&mut ws, serde_json::json!({ "type": "stop" }));
    assert_eq!(
        next(&mut ws),
        serde_json::json!({ "type": "stopped", "saved": [] })
    );
}

/// A message the server does not understand is answered, not dropped, and
/// the session goes on.
#[test]
fn a_message_not_understood_is_answered_with_an_error() {
    let (addr, _dir) = prompting(&[]);
    let mut ws = session(addr);
    send(&mut ws, serde_json::json!({ "type": "rewind" }));
    let error = next(&mut ws);
    assert_eq!(error["type"], "error", "{error}");
    send(&mut ws, serde_json::json!({ "type": "stop" }));
    assert_eq!(next(&mut ws)["type"], "stopped");
}

/// One prompter, one reader: a second session is refused while the first
/// is open, and allowed once it closes.
#[test]
fn one_session_at_a_time() {
    let (addr, _dir) = prompting(&[]);
    let mut first = session(addr);
    let stream = TcpStream::connect(addr).unwrap();
    let url = format!("ws://{addr}/api/v1/session");
    match tungstenite::client(url.as_str(), stream) {
        Err(tungstenite::HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            assert_eq!(response.status(), 409)
        }
        other => panic!("a second session was not refused: {:?}", other.map(|_| ())),
    }
    first.close(None).unwrap();
    while first.read().is_ok() {}
    // The server lets go of the session once it has seen the close.
    let mut again = (0..40)
        .find_map(|_| {
            let stream = TcpStream::connect(addr).unwrap();
            let url = format!("ws://{addr}/api/v1/session");
            let opened = tungstenite::client(url.as_str(), stream).ok();
            if opened.is_none() {
                std::thread::sleep(Duration::from_millis(50));
            }
            opened
        })
        .expect("a session once the first closed")
        .0;
    send(&mut again, serde_json::json!({ "type": "stop" }));
    assert_eq!(next(&mut again)["type"], "stopped");
}

/// A recognizer that fails as a native library can: by panicking.
struct Panics;

impl Recognizer for Panics {
    fn listen(&mut self, _: &[f32]) -> Heard {
        panic!("the recognizer fell over")
    }

    fn reset(&mut self) {}
}

/// A session that ends in a panic still lets go: the next reader gets a
/// session, not "already open" until the prompter is restarted.
#[test]
fn a_session_that_panics_lets_go_of_the_prompter() {
    let dir = teleprompt_testkit::test_dir("prompt-panics");
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    let addr = listener.local_addr().unwrap();
    let prompt = Prompt {
        name: "tour.md".into(),
        lines: LINES.iter().map(|l| l.to_string()).collect(),
        ids: IDS.iter().map(|l| LineId::from(*l)).collect(),
        shots: shots(),
        clips: dir.to_path_buf(),
        takes: dir.join("takes"),
    };
    std::thread::spawn(move || prompt_on(listener, prompt, Panics));

    let mut first = session(addr);
    send(
        &mut first,
        serde_json::json!({ "type": "start", "from": 0, "rate": 16000 }),
    );
    next(&mut first);
    send_audio(&mut first, 1600);
    while first.read().is_ok() {}

    let again = (0..40).find_map(|_| {
        let stream = TcpStream::connect(addr).unwrap();
        let url = format!("ws://{addr}/api/v1/session");
        let opened = tungstenite::client(url.as_str(), stream).ok();
        if opened.is_none() {
            std::thread::sleep(Duration::from_millis(50));
        }
        opened
    });
    assert!(
        again.is_some(),
        "the prompter still thinks a session is open"
    );
}

/// The session route is a socket; a plain request is told so.
#[test]
fn the_session_route_needs_a_websocket() {
    let (addr, _dir) = prompting(&[]);
    let response = String::from_utf8_lossy(&get(addr, "/api/v1/session")).to_string();
    assert!(response.starts_with("HTTP/1.1 426"), "{response}");
}

/// The script lists the lines as the aligner counts their words, which are
/// recorded, and each shot with its cue and clip; one never captured has
/// none.
#[test]
fn the_script_lists_lines_and_shots() {
    let (addr, _clips) = prompting(&[]);
    let script = json_at(addr, "/api/v1/script");
    let captured = Hash::of(b"welcome-a#0");
    assert_eq!(
        script,
        serde_json::json!({
            "name": "tour.md",
            "lines": [
                { "id": "welcome", "text": LINES[0], "recorded": false, "stale": false, "said": null, "said_diff": [] },
                { "id": "deploy", "text": LINES[1], "recorded": false, "stale": false, "said": null, "said_diff": [] },
            ],
            "shots": [
                { "shot": "intro#0", "at": { "line": 0, "word": 0 }, "clip": null },
                { "shot": "welcome-a#0", "at": { "line": 0, "word": 3 },
                  "clip": format!("/api/v1/clips/{captured}.mp4") },
                { "shot": "welcome-b#0", "at": { "line": 1, "word": 0 }, "clip": null },
            ],
        })
    );
}

/// A cued shot's clip is served as video; nothing else in the cache is.
#[test]
fn a_cued_clip_is_served_and_nothing_else() {
    let (addr, clips) = prompting(&[]);
    let captured = Hash::of(b"welcome-a#0");
    let response = get(addr, &format!("/api/v1/clips/{captured}.mp4"));
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(text.contains("Content-Type: video/mp4"), "{text}");
    assert!(response.ends_with(CLIP), "{text}");

    let stray = Hash::of(b"not a cued shot");
    std::fs::write(clips.join(format!("{stray}.mp4")), CLIP).unwrap();
    for path in [
        format!("/api/v1/clips/{stray}.mp4"),
        "/api/v1/clips/../secret.mp4".to_string(),
    ] {
        let response = String::from_utf8_lossy(&get(addr, &path)).to_string();
        assert!(response.starts_with("HTTP/1.1 404"), "{path}: {response}");
    }
}

/// The API is versioned; the unversioned routes of before are gone, and an
/// unknown version is not served.
#[test]
fn only_version_1_is_served() {
    let (addr, _clips) = prompting(&[]);
    for path in ["/script.json", "/api/v2/script", "/api/script"] {
        let response = String::from_utf8_lossy(&get(addr, path)).to_string();
        assert!(response.starts_with("HTTP/1.1 404"), "{path}: {response}");
    }
}

/// The page is served at the root, and it is the one that reads the script
/// and opens the session.
#[test]
fn the_prompter_page_is_served_at_the_root() {
    let (addr, _clips) = prompting(&[]);
    let page = request(addr, "GET", "/", &[]);
    assert!(
        page.starts_with("<!doctype html>"),
        "{}",
        &page[..page.len().min(80)]
    );
    assert!(page.contains(r#"const API = "/api/v1";"#));
    assert!(page.contains("${API}/script") && page.contains("${API}/session"));
}

/// Browsers ask for an icon on every visit; the answer is "none", not an
/// error in the page's console.
#[test]
fn the_icon_request_is_answered_with_nothing() {
    let (addr, _clips) = prompting(&[]);
    let response = String::from_utf8_lossy(&get(addr, "/favicon.ico")).to_string();
    assert!(response.starts_with("HTTP/1.1 204"), "{response}");
}

fn tp_prompt(tag: &str, extra: &[&str]) -> std::process::Output {
    let dir = teleprompt_testkit::test_dir(tag);
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::process::Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(&dir)
        .args(["prompt", "scripts/demo.md"])
        .args(extra)
        .output()
        .unwrap()
}

/// The recognizer is opt-in at build time; a build without it says so and
/// how to get one, rather than serving a page that can never follow.
#[cfg(not(feature = "listen"))]
#[test]
fn without_a_recognizer_the_command_says_how_to_get_one() {
    let out = tp_prompt("prompt-unbuilt", &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("--features listen"), "{stderr}");
}

/// teleprompt downloads nothing: without a model it names the one to get.
#[cfg(feature = "listen")]
#[test]
fn without_a_model_the_command_says_which_to_download() {
    let out = tp_prompt("prompt-nomodel", &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("sherpa-onnx-streaming-zipformer-en"),
        "{stderr}"
    );
}

const TOUR: &str = "\
---
teleprompt: 1
scene:
  demo:
    adapter: mock
---

# A tour

```teleprompt scene=demo
wait 800ms
```

Welcome to Acme. Let me show you around. {#welcome}

```teleprompt scene=demo
wait 800ms
```

Deployment is one command, and it streams progress. {#deploy}

```teleprompt scene=demo policy=concurrent cue=\"streams progress\"
wait 800ms
```

That is all there is. {#end}

```teleprompt scene=demo policy=concurrent
wait 800ms
```
";

fn compiled_tour(tag: &str) -> teleprompt_compile::CompileOutput {
    let dir = teleprompt_testkit::test_dir(tag);
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/tour.md");
    std::fs::write(&script, TOUR).unwrap();
    let project = teleprompt_cli::project::Project::discover(&dir).unwrap();
    teleprompt_cli::cmd::check::compile_script(&project, &script, "en")
        .unwrap_or_else(|e| panic!("{e:?}"))
        .0
}

/// Each shot starts where the reader's voice puts it, by the same policy the
/// video uses: before any line, at the top; under `hold`, once its line is
/// said; `concurrent`, with the line's first word, or on its cue.
#[test]
fn each_shot_is_cued_where_its_policy_starts_it() {
    use teleprompt_listen::Position;
    let compiled = compiled_tour("prompt-cues");
    let cues = teleprompt_prompter::shot_cues(&compiled);
    let at: Vec<Position> = cues.iter().map(|c| c.at).collect();
    let pos = |line, word| Position { line, word };
    assert_eq!(at, [pos(0, 0), pos(1, 0), pos(1, 7), pos(2, 1)]);

    let shots: Vec<&str> = compiled
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.action.as_ref())
        .map(|a| a.shot.as_str())
        .collect();
    let cued: Vec<&str> = cues.iter().map(|c| c.shot.as_str()).collect();
    assert_eq!(cued, shots, "every shot, in timeline order");
}

/// An example from `docs/api/v1/examples`: the contract the app is tested
/// against too.
fn example(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/../../docs/api/v1/examples/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn send_example(ws: &mut Socket, name: &str) {
    send(ws, example(name));
}

/// The documented examples are what the server says and accepts.
#[test]
fn the_examples_are_what_the_server_says() {
    let (addr, _clips) = prompting(&["deployment is"]);
    assert_eq!(json_at(addr, "/api/v1/script"), example("script.json"));
    let mut ws = session(addr);
    send_example(&mut ws, "start.json");
    assert_eq!(next(&mut ws), example("reached.json"));
    ws.send(tungstenite::Message::text(r#"{"type":"rewind"}"#))
        .unwrap();
    assert_eq!(next(&mut ws), example("error.json"));
}

/// 16 kHz audio: `spans` of (seconds, loud), as the bytes the socket takes.
fn audio(spans: &[(f32, bool)]) -> Vec<u8> {
    let mut out = Vec::new();
    for &(seconds, loud) in spans {
        for i in 0..(seconds * 16_000.0) as usize {
            let v = if loud {
                0.3 * (i as f32 * 0.2).sin()
            } else {
                0.0
            };
            out.extend(v.to_le_bytes());
        }
    }
    out
}

/// A take read over the socket and stopped answers as the example does.
#[test]
fn a_take_stopped_over_the_socket_answers_as_the_example() {
    const LINE_0: &str = "welcome to acme let me show you around";
    let mut heard = vec![""; 5];
    heard.extend(["welcome to"; 6]);
    heard.extend([LINE_0; 7]);
    heard.extend([""; 5]);
    heard.extend(["deployment is"; 7]);
    let (addr, _dir) = prompting(&heard);
    let mut ws = session(addr);
    send(
        &mut ws,
        serde_json::json!({ "type": "start", "from": 0, "rate": 16000 }),
    );
    let take = audio(&[(0.3, false), (1.2, true), (0.8, false), (0.7, true)]);
    for chunk in take.chunks(1_600 * 4) {
        ws.send(tungstenite::Message::binary(chunk.to_vec()))
            .unwrap();
    }
    send_example(&mut ws, "stop.json");
    let stopped = loop {
        let message = next(&mut ws);
        if message["type"] != "reached" {
            break message;
        }
    };
    assert_eq!(stopped, example("stopped.json"));
}

/// Launched by an app with `--format json`, the command says where it
/// listens, on stdout, as the example does.
#[test]
fn the_listening_event_is_the_example() {
    let addr: SocketAddr = "127.0.0.1:7879".parse().unwrap();
    assert_eq!(
        teleprompt_cli::cmd::prompt::listening_event(addr),
        example("listening.json")
    );
}

/// The page is set in the prompters' typeface, served with it, so it looks
/// the same on a machine that has never installed it.
#[test]
fn the_page_s_typeface_is_served() {
    let (addr, _clips) = prompting(&[]);
    let response = get(addr, "/fonts/atkinson-hyperlegible-next.woff2");
    let head = String::from_utf8_lossy(&response[..response.len().min(300)]).to_string();
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(head.contains("Content-Type: font/woff2"), "{head}");
    let page = request(addr, "GET", "/", &[]);
    assert!(page.contains("/fonts/atkinson-hyperlegible-next.woff2"));
}

/// A script edited while it is read: the next fetch of it places its
/// shots again, as the edit left them.
#[test]
fn an_edited_script_s_shots_are_placed_again_when_it_is_fetched() {
    let clips = teleprompt_testkit::test_dir("prompt-reload");
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    let addr = listener.local_addr().unwrap();
    let prompt = Prompt {
        name: "tour.md".into(),
        lines: LINES.iter().map(|l| l.to_string()).collect(),
        ids: IDS.iter().map(|l| LineId::from(*l)).collect(),
        shots: shots(),
        clips: clips.to_path_buf(),
        takes: clips.join("takes"),
    };
    // Edited once: the second shot moved to the first line's sixth word.
    let edited = std::sync::Mutex::new(Some({
        let mut moved = prompt.clone();
        moved.shots[1].at = Position { line: 0, word: 5 };
        moved
    }));
    let reload: teleprompt_cli::cmd::prompt::Reload =
        Box::new(move || edited.lock().unwrap().take());
    std::thread::spawn(move || {
        teleprompt_cli::cmd::prompt::prompt_watching(
            listener,
            prompt,
            Scripted(Default::default(), Default::default()),
            Some(teleprompt_cli::cmd::prompt::Edits {
                reload,
                keep_said: Box::new(|_| Ok(())),
            }),
        )
    });
    let script = json_at(addr, "/api/v1/script");
    assert_eq!(script["shots"][1]["at"]["word"], 5, "{script}");
    // Not edited since: as it was left.
    let again = json_at(addr, "/api/v1/script");
    assert_eq!(again["shots"][1]["at"]["word"], 5, "{again}");
}

/// Serves `prompt` with `keep_said` for the edits, and no reload.
fn prompting_with_keep(
    keep_said: teleprompt_cli::cmd::prompt::KeepSaid,
) -> (SocketAddr, teleprompt_testkit::TestDir) {
    let clips = teleprompt_testkit::test_dir("prompt-keep");
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    let addr = listener.local_addr().unwrap();
    let prompt = Prompt {
        name: "tour.md".into(),
        lines: LINES.iter().map(|l| l.to_string()).collect(),
        ids: IDS.iter().map(|l| LineId::from(*l)).collect(),
        shots: shots(),
        clips: clips.to_path_buf(),
        takes: clips.join("takes"),
    };
    let edits = teleprompt_cli::cmd::prompt::Edits {
        reload: Box::new(|| None),
        keep_said,
    };
    std::thread::spawn(move || {
        teleprompt_cli::cmd::prompt::prompt_watching(
            listener,
            prompt,
            Scripted(Default::default(), Default::default()),
            Some(edits),
        )
    });
    (addr, clips)
}

/// A line whose take says other words comes with them, and with the line
/// against them word by word, for a prompter to show before keeping them.
#[test]
fn a_line_said_otherwise_comes_with_what_keeping_it_changes() {
    let dir = teleprompt_testkit::test_dir("prompt-said-diff");
    let pcm = teleprompt_voice::Pcm {
        sample_rate: 16_000,
        channels: 1,
        samples: vec![0; 16_000],
    };
    teleprompt_voice::takes::Takes::load(&dir.join("takes"))
        .unwrap()
        .save_heard("deploy", LINES[1], "deployment is just one command", &pcm)
        .unwrap();
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    let addr = listener.local_addr().unwrap();
    let prompt = Prompt {
        name: "tour.md".into(),
        lines: LINES.iter().map(|l| l.to_string()).collect(),
        ids: IDS.iter().map(|l| LineId::from(*l)).collect(),
        shots: shots(),
        clips: dir.to_path_buf(),
        takes: dir.join("takes"),
    };
    let recognizer = Scripted(Default::default(), Default::default());
    std::thread::spawn(move || prompt_on(listener, prompt, recognizer));

    let line = &json_at(addr, "/api/v1/script")["lines"][1];
    assert_eq!(line["said"], "Deployment is just one command.", "{line}");
    assert_eq!(
        line["said_diff"],
        serde_json::json!([
            { "kind": "same", "words": "Deployment is" },
            { "kind": "new", "words": "just" },
            { "kind": "same", "words": "one command." },
        ])
    );
}

/// Keeping what was said on a line rewords it, and says so; the page
/// fetches the script again for the new words.
#[test]
fn keeping_what_was_said_rewords_the_line() {
    let kept = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = kept.clone();
    let (addr, _dir) = prompting_with_keep(Box::new(move |line| {
        seen.lock().unwrap().push(line.to_string());
        Ok(())
    }));
    let mut ws = session(addr);
    send_example(&mut ws, "keep_said.json");
    assert_eq!(next(&mut ws), example("kept_said.json"));
    assert_eq!(*kept.lock().unwrap(), ["welcome"]);
}

#[test]
fn keeping_what_was_said_that_fails_says_why() {
    let (addr, _dir) = prompting_with_keep(Box::new(|line| Err(format!("no take for `{line}`"))));
    let mut ws = session(addr);
    send(
        &mut ws,
        serde_json::json!({ "type": "keep_said", "line": "deploy" }),
    );
    assert_eq!(
        next(&mut ws),
        serde_json::json!({ "type": "error", "message": "no take for `deploy`" })
    );
}

/// A prompter with no script file behind it has nothing to reword.
#[test]
fn keeping_what_was_said_without_a_script_file_is_an_error() {
    let (addr, _dir) = prompting(&[]);
    let mut ws = session(addr);
    send(
        &mut ws,
        serde_json::json!({ "type": "keep_said", "line": "welcome" }),
    );
    let answer = next(&mut ws);
    assert_eq!(answer["type"], "error", "{answer}");
}

/// Opens the session socket as a page at `origin` would.
fn session_from(addr: SocketAddr, origin: &str) -> Result<Socket, tungstenite::Error> {
    use tungstenite::client::IntoClientRequest;
    let stream = TcpStream::connect(addr).unwrap();
    let mut request = format!("ws://{addr}/api/v1/session")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", origin.parse().unwrap());
    tungstenite::client(request, stream)
        .map(|(ws, _)| ws)
        .map_err(|e| match e {
            tungstenite::HandshakeError::Failure(e) => e,
            tungstenite::HandshakeError::Interrupted(_) => panic!("a blocking socket"),
        })
}

/// A browser lets any page open a socket to loopback, so the session is
/// refused to a page this server did not serve: it could record over a
/// take, or reword the script.
#[test]
fn a_session_from_another_sites_page_is_refused() {
    let (addr, _dir) = prompting(&[]);
    match session_from(addr, "https://example.com") {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 403),
        other => panic!("expected 403, got {:?}", other.map(|_| ())),
    }
    let own = session_from(addr, &format!("http://{addr}"));
    assert!(own.is_ok(), "the prompter's own page opens it");
}

/// A name that resolves to loopback only after the page loaded (DNS
/// rebinding) is not this machine's name: nothing is served to it.
#[test]
fn a_request_for_another_host_is_refused() {
    let (addr, _dir) = prompting(&[]);
    let mut s = TcpStream::connect(addr).unwrap();
    write!(
        s,
        "GET /api/v1/script HTTP/1.1\r\nHost: rebound.example:{}\r\n\r\n",
        addr.port()
    )
    .unwrap();
    let mut response = String::new();
    s.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
}
