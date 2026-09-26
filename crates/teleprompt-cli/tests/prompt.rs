//! `teleprompt prompt`'s REST API, driven over a real socket with a
//! recognizer that hears what the test says. What the prompter does is
//! tested in `teleprompt-prompter`; these pin the routes and the JSON.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use teleprompt_cli::cmd::prompt::prompt_on;
use teleprompt_core::Hash;
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
        shot: shot.to_string(),
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
        lines: LINES.iter().map(|l| l.to_string()).collect(),
        ids: IDS.iter().map(|l| l.to_string()).collect(),
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

fn samples(n: usize) -> Vec<u8> {
    vec![0u8; n * 4]
}

/// The page posts the microphone's samples as they come and reads back
/// where the reader is.
#[test]
fn posting_audio_answers_where_the_reader_is() {
    let (addr, _clips) = prompting(&["welcome to"]);
    let body = request(addr, "POST", "/listen", &samples(1600));
    let at: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (at["line"].as_u64(), at["word"].as_u64()),
        (Some(0), Some(2))
    );
}

/// The page shows the lines exactly as the aligner counts their words, so a
/// position picks out the word on screen.
#[test]
fn the_script_is_served_as_the_lines_it_follows() {
    let (addr, _clips) = prompting(&[]);
    let body = request(addr, "GET", "/script.json", &[]);
    let script: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(script["lines"], serde_json::json!(LINES));
}

/// The page marks where each shot starts, and knows which have a clip to
/// play; one that was never captured is shown as missing, not skipped.
#[test]
fn the_script_names_each_shot_where_it_starts_and_its_clip() {
    let (addr, _clips) = prompting(&[]);
    let body = request(addr, "GET", "/script.json", &[]);
    let script: serde_json::Value = serde_json::from_str(&body).unwrap();
    let captured = Hash::of(b"welcome-a#0");
    assert_eq!(
        script["shots"],
        serde_json::json!([
            { "shot": "intro#0", "at": { "line": 0, "word": 0 }, "clip": null },
            { "shot": "welcome-a#0", "at": { "line": 0, "word": 3 },
              "clip": format!("/clips/{captured}.mp4") },
            { "shot": "welcome-b#0", "at": { "line": 1, "word": 0 }, "clip": null },
        ])
    );
}

/// A captured clip is served as video; nothing else in the cache is.
#[test]
fn a_cued_clip_is_served_and_nothing_else() {
    let (addr, clips) = prompting(&[]);
    let captured = Hash::of(b"welcome-a#0");
    let mut s = TcpStream::connect(addr).unwrap();
    write!(
        s,
        "GET /clips/{captured}.mp4 HTTP/1.1\r\nHost: localhost\r\n\r\n"
    )
    .unwrap();
    let mut response = Vec::new();
    s.read_to_end(&mut response).unwrap();
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(text.contains("Content-Type: video/mp4"), "{text}");
    assert!(response.ends_with(CLIP), "{text}");

    let stray = Hash::of(b"not a cued shot");
    std::fs::write(clips.join(format!("{stray}.mp4")), CLIP).unwrap();
    for path in [
        format!("/clips/{stray}.mp4"),
        "/clips/../secret.mp4".to_string(),
    ] {
        let mut s = TcpStream::connect(addr).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut response = String::new();
        s.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 404"), "{path}: {response}");
    }
}

/// The page is served at the root, and it is the one that reads the script
/// and posts the microphone.
#[test]
fn the_prompter_page_is_served_at_the_root() {
    let (addr, _clips) = prompting(&[]);
    let page = request(addr, "GET", "/", &[]);
    assert!(
        page.starts_with("<!doctype html>"),
        "{}",
        &page[..page.len().min(80)]
    );
    assert!(page.contains("/script.json") && page.contains("/listen"));
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

/// Browsers ask for an icon on every visit; the answer is "none", not an
/// error in the page's console.
#[test]
fn the_icon_request_is_answered_with_nothing() {
    let (addr, _clips) = prompting(&[]);
    let mut s = TcpStream::connect(addr).unwrap();
    write!(s, "GET /favicon.ico HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = String::new();
    s.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 204"), "{response}");
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

/// The page sends the microphone at its own rate; the recognizer hears it
/// at the rate it was made for.
#[test]
fn audio_at_the_microphones_rate_reaches_the_recognizer_at_its_own() {
    let (addr, _dir, heard) = prompting_counted(&[]);
    request(addr, "POST", "/listen?rate=48000", &samples(4_800));
    request(addr, "POST", "/listen?rate=48000", &samples(4_800));
    let n = heard.load(Ordering::SeqCst);
    assert!((3_100..=3_200).contains(&n), "{n} samples at 16 kHz");
}

/// A take can start at a line, to read it again; the shots before it do not
/// play.
#[test]
fn a_take_can_start_at_a_line() {
    let (addr, _dir) = prompting(&["deployment is"]);
    let top: serde_json::Value =
        serde_json::from_str(&request(addr, "POST", "/start?from=1", &[])).unwrap();
    assert_eq!(
        top,
        serde_json::json!({ "line": 1, "word": 0, "play": ["welcome-b#0"] })
    );
    let at: serde_json::Value =
        serde_json::from_str(&request(addr, "POST", "/listen", &samples(1600))).unwrap();
    assert_eq!(
        (at["line"].as_u64(), at["word"].as_u64()),
        (Some(1), Some(2))
    );
}

/// Stopping answers the ids of the lines kept, and the script says which
/// lines are recorded.
#[test]
fn stopping_answers_the_lines_kept() {
    let (addr, _dir) = prompting(&[]);
    let stopped: serde_json::Value =
        serde_json::from_str(&request(addr, "POST", "/stop", &[])).unwrap();
    assert_eq!(stopped, serde_json::json!({ "saved": [] }));
    let script: serde_json::Value =
        serde_json::from_str(&request(addr, "GET", "/script.json", &[])).unwrap();
    assert_eq!(script["recorded"], serde_json::json!([false, false]));
}
