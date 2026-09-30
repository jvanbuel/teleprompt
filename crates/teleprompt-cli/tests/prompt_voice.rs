//! `teleprompt prompt --voice`: a script read by its voice rather than
//! followed by ear. Runs in any build, without a speech model; the script
//! says how each line sounds, a line's audio is made when asked for, and
//! a line can be reworded or told how to sound over the session socket.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

mod browser;

struct Served {
    child: Child,
    addr: SocketAddr,
    dir: teleprompt_testkit::TestDir,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The scaffolded project, prompted in voice mode on a port the OS picks.
fn voiced(tag: &str) -> Served {
    let dir = teleprompt_testkit::test_dir(tag);
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(&dir)
        .args([
            "--format",
            "json",
            "prompt",
            "scripts/demo.md",
            "--voice",
            "--port",
            "0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut first = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    let event: serde_json::Value =
        serde_json::from_str(&first).unwrap_or_else(|e| panic!("{e}: {first:?}"));
    let addr = event["url"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://")
        .parse()
        .unwrap();
    Served { child, addr, dir }
}

/// A GET: the status code and the body.
fn get(addr: SocketAddr, path: &str) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(addr).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = Vec::new();
    s.read_to_end(&mut response).unwrap();
    let at = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let status = String::from_utf8_lossy(&response[9..12]).parse().unwrap();
    (status, response[at + 4..].to_vec())
}

fn script(addr: SocketAddr) -> serde_json::Value {
    let (status, body) = get(addr, "/api/v1/script");
    assert_eq!(status, 200);
    serde_json::from_slice(&body).unwrap()
}

fn line<'a>(script: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    script["lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == id)
        .unwrap_or_else(|| panic!("no line {id} in {script}"))
}

type Socket = tungstenite::WebSocket<TcpStream>;

fn session(addr: SocketAddr) -> Socket {
    let stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let url = format!("ws://{addr}/api/v1/session");
    tungstenite::client(url.as_str(), stream).unwrap().0
}

fn ask(ws: &mut Socket, message: serde_json::Value) -> serde_json::Value {
    ws.send(tungstenite::Message::text(message.to_string()))
        .unwrap();
    loop {
        if let tungstenite::Message::Text(text) = ws.read().unwrap() {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

#[test]
fn a_voiced_script_says_who_reads_it_and_how_long_it_runs() {
    let served = voiced("prompt-voice-script");
    let s = script(served.addr);
    assert_eq!(s["voice"]["listens"], false, "{s}");
    assert!(s["voice"]["name"].as_str().unwrap().contains("null"), "{s}");
    assert!(s["length_ms"].as_u64().unwrap() > 0, "{s}");
    let welcome = line(&s, "welcome");
    assert_eq!(welcome["audio"]["source"], "voice", "{welcome}");
    assert_eq!(welcome["audio"]["ready"], false, "{welcome}");
    assert_eq!(welcome["audio"]["url"], "/api/v1/voice/welcome.wav");
    assert!(welcome["instruct"].is_null(), "{welcome}");
}

#[test]
fn a_lines_audio_is_made_when_asked_for_and_then_timed() {
    let served = voiced("prompt-voice-audio");
    let (status, wav) = get(served.addr, "/api/v1/voice/welcome.wav");
    assert_eq!(status, 200);
    assert_eq!(&wav[..4], b"RIFF");
    let s = script(served.addr);
    let welcome = line(&s, "welcome");
    let audio = &welcome["audio"];
    assert_eq!(audio["ready"], true, "{welcome}");
    let duration = audio["duration_ms"].as_u64().unwrap();
    let words = audio["words"].as_array().unwrap();
    let text = welcome["text"].as_str().unwrap();
    assert_eq!(words.len(), text.split_whitespace().count(), "{welcome}");
    assert_eq!(words[0], 0);
    assert!(words.iter().all(|w| w.as_u64().unwrap() < duration));
    // Made again on request: still the line's audio.
    let (status, again) = get(served.addr, "/api/v1/voice/welcome.wav?fresh=1");
    assert_eq!((status, &again[..4]), (200, &b"RIFF"[..]));
    assert_eq!(get(served.addr, "/api/v1/voice/nope.wav").0, 404);
}

#[test]
fn a_line_is_reworded_and_told_how_to_sound_over_the_socket() {
    let served = voiced("prompt-voice-edit");
    let mut ws = session(served.addr);
    assert_eq!(
        ask(
            &mut ws,
            serde_json::json!({ "type": "reword", "line": "welcome", "text": "Hello, and welcome." })
        ),
        serde_json::json!({ "type": "edited", "line": "welcome" })
    );
    assert_eq!(
        ask(
            &mut ws,
            serde_json::json!({ "type": "instruct", "line": "welcome", "text": "brightly" })
        ),
        serde_json::json!({ "type": "edited", "line": "welcome" })
    );
    let s = script(served.addr);
    let welcome = line(&s, "welcome");
    assert_eq!(welcome["text"], "Hello, and welcome.", "{welcome}");
    assert_eq!(welcome["instruct"], "brightly", "{welcome}");
    let md = std::fs::read_to_string(served.dir.join("scripts/demo.md")).unwrap();
    assert!(
        md.contains("Hello, and welcome. {#welcome voice.instruct=brightly}"),
        "{md}"
    );
    // An edit that cannot be made says why, and changes nothing.
    let refused = ask(
        &mut ws,
        serde_json::json!({ "type": "reword", "line": "nope", "text": "x" }),
    );
    assert_eq!(refused["type"], "error", "{refused}");
}

#[test]
fn a_voiced_prompter_records_no_takes() {
    let served = voiced("prompt-voice-no-takes");
    let mut ws = session(served.addr);
    let answer = ask(
        &mut ws,
        serde_json::json!({ "type": "start", "from": 0, "rate": 16000 }),
    );
    assert_eq!(answer["type"], "error", "{answer}");
}

fn example(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/../../docs/api/v1/examples/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// The keys of an object, and of the objects in it, as a sorted list of
/// paths: the example's shape without its values.
fn shape(value: &serde_json::Value, at: &str, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, v) in map {
                let path = format!("{at}.{key}");
                out.push(path.clone());
                shape(v, &path, out);
            }
        }
        serde_json::Value::Array(items) => {
            if let Some(first) = items.iter().find(|i| i.is_object()) {
                shape(first, &format!("{at}[]"), out);
            }
        }
        _ => {}
    }
}

#[test]
fn the_voiced_examples_are_what_the_server_says_and_takes() {
    let served = voiced("prompt-voice-examples");
    let (mut said, mut documented) = (Vec::new(), Vec::new());
    get(served.addr, "/api/v1/voice/welcome.wav");
    shape(&script(served.addr), "", &mut said);
    shape(&example("voiced_script.json"), "", &mut documented);
    said.sort();
    said.dedup();
    documented.sort();
    documented.dedup();
    assert_eq!(said, documented);
    let mut ws = session(served.addr);
    let mut reword = example("reword.json");
    reword["line"] = "welcome".into();
    let mut edited = example("edited.json");
    edited["line"] = "welcome".into();
    assert_eq!(ask(&mut ws, reword), edited);
    assert_eq!(ask(&mut ws, example("instruct.json")), edited);
}

/// The page, served by a voiced prompter, plays rather than records, and
/// marks each line by whether its voice has made it.
#[test]
fn the_page_reads_with_the_voice() {
    let Some(chrome) = browser::chromium() else {
        eprintln!("skipped: no Chromium");
        return;
    };
    let served = voiced("prompt-voice-page");
    get(served.addr, "/api/v1/voice/welcome.wav");
    let page = browser::dom(
        &chrome,
        &format!("http://{}/", served.addr),
        1200,
        800,
        4000,
    );
    assert!(page.contains(r#"id="record""#), "{page}");
    let record = page.split(r#"id="record""#).nth(1).unwrap();
    assert!(record.contains(">Play<"), "{record}");
    assert!(page.contains(r#"class="line voiced"#), "{page}");
}
