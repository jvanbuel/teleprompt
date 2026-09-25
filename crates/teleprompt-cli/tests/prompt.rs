//! `teleprompt prompt`'s server, driven over a real socket with a
//! recognizer that hears what the test says.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};

use teleprompt_cli::cmd::prompt::prompt_on;
use teleprompt_listen::{Heard, Recognizer};

struct Scripted(VecDeque<&'static str>);

impl Recognizer for Scripted {
    fn listen(&mut self, _samples: &[f32]) -> Heard {
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

/// A prompter served on a port the OS picks, hearing `heard` in turn.
fn prompting(heard: &[&'static str]) -> SocketAddr {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    let addr = listener.local_addr().unwrap();
    let lines = LINES.iter().map(|l| l.to_string()).collect();
    let recognizer = Scripted(heard.iter().copied().collect());
    std::thread::spawn(move || prompt_on(listener, lines, recognizer));
    addr
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
    let addr = prompting(&["welcome to"]);
    let body = request(addr, "POST", "/listen", &samples(1600));
    let at: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(at, serde_json::json!({ "line": 0, "word": 2 }));
}

/// The page shows the lines exactly as the aligner counts their words, so a
/// position picks out the word on screen.
#[test]
fn the_script_is_served_as_the_lines_it_follows() {
    let addr = prompting(&[]);
    let body = request(addr, "GET", "/script.json", &[]);
    let script: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(script, serde_json::json!({ "lines": LINES }));
}

/// The page is served at the root, and it is the one that reads the script
/// and posts the microphone.
#[test]
fn the_prompter_page_is_served_at_the_root() {
    let addr = prompting(&[]);
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
    let addr = prompting(&[]);
    let mut s = TcpStream::connect(addr).unwrap();
    write!(s, "GET /favicon.ico HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = String::new();
    s.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 204"), "{response}");
}

/// Clicking start begins a take from the top, whatever the last one
/// reached, and says so, so the page redraws from there.
#[test]
fn starting_a_take_goes_back_to_the_top() {
    let addr = prompting(&["welcome to acme"]);
    request(addr, "POST", "/listen", &samples(1600));
    let body = request(addr, "POST", "/start", &[]);
    let at: serde_json::Value = serde_json::from_str(&body).expect(&body);
    assert_eq!(at, serde_json::json!({ "line": 0, "word": 0 }));
}
