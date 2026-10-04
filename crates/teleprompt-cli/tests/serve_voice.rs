//! `teleprompt serve --voice`: a script read by its voice rather than
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
    voiced_script(tag, "demo.md", None)
}

/// [`voiced`], prompting `scripts/<name>`, written as `source` if given.
fn voiced_script(tag: &str, name: &str, source: Option<&str>) -> Served {
    let dir = teleprompt_testkit::test_dir(tag);
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    if let Some(source) = source {
        std::fs::write(dir.join("scripts").join(name), source).unwrap();
    }
    let script = format!("scripts/{name}");
    let mut child = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(&dir)
        .args([
            "--format",
            "json",
            "serve",
            script.as_str(),
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
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
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
    let mut moved = example("edited_block.json");
    moved["block"] = "the-loop-a".into();
    for name in ["cue.json", "stretch.json", "hold.json"] {
        let mut edit = example(name);
        edit["block"] = "the-loop-a".into();
        assert_eq!(ask(&mut ws, edit), moved, "{name}");
    }
    let mut to = example("move.json");
    (to["block"], to["after"]) = ("the-loop-a".into(), "welcome".into());
    assert_eq!(ask(&mut ws, to), moved);
    assert_eq!(
        ask(&mut ws, example("undo_edit.json")),
        example("edit_undone.json")
    );
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

/// The script as the scheduler lays it out: each line and shot in time,
/// for a prompter to draw the shots on the glass and drag them.
#[test]
fn the_script_comes_with_its_timeline() {
    let served = voiced("prompt-voice-timeline");
    let s = script(served.addr);
    let timeline = &s["timeline"];
    assert!(timeline["duration_ms"].as_u64().unwrap() > 0, "{timeline}");
    let lines = timeline["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2, "{timeline}");
    assert_eq!(lines[0]["id"], "welcome");
    assert!(lines[0]["start_ms"].as_u64() < lines[0]["end_ms"].as_u64());
    let shots = timeline["shots"].as_array().unwrap();
    assert_eq!(shots.len(), 2, "{timeline}");
    assert_eq!(shots[1]["shot"], "the-loop-a#0");
    assert_eq!(shots[1]["block"], "the-loop-a");
    assert_eq!(shots[1]["scene"], "mock");
    assert_eq!(shots[1]["line"], "the-loop");
    assert_eq!(shots[1]["timed"], true);
    assert!(shots[1]["start_ms"].as_u64() < shots[1]["end_ms"].as_u64());
}

fn demo(served: &Served) -> String {
    std::fs::read_to_string(served.dir.join("scripts/demo.md")).unwrap()
}

/// A shot dragged on the glass: cued to a word, held after its line,
/// moved to another line or stretched; and the last edit undone.
#[test]
fn a_shot_is_moved_over_the_socket_and_the_edit_undone() {
    let served = voiced("prompt-voice-shots");
    let before = demo(&served);
    let mut ws = session(served.addr);
    let cue = serde_json::json!({ "type": "cue", "block": "the-loop-a", "word": 3 });
    assert_eq!(
        ask(&mut ws, cue),
        serde_json::json!({ "type": "edited", "block": "the-loop-a" })
    );
    assert!(demo(&served).contains("cue="), "{}", demo(&served));
    let stretch = serde_json::json!({ "type": "stretch", "block": "welcome-a", "by": 2.0 });
    assert_eq!(ask(&mut ws, stretch)["type"], "edited");
    let hold = serde_json::json!({ "type": "hold", "block": "the-loop-a" });
    assert_eq!(ask(&mut ws, hold)["type"], "edited");
    let moved = serde_json::json!({ "type": "move", "block": "welcome-a", "after": "the-loop", "word": null });
    assert_eq!(ask(&mut ws, moved)["type"], "edited");
    for _ in 0..4 {
        let undone = ask(&mut ws, serde_json::json!({ "type": "undo_edit" }));
        assert_eq!(undone, serde_json::json!({ "type": "edit_undone" }));
    }
    assert_eq!(demo(&served), before);
    let nothing = ask(&mut ws, serde_json::json!({ "type": "undo_edit" }));
    assert_eq!(nothing["type"], "error", "{nothing}");
    // A block the script lacks is refused, and nothing is kept to undo.
    let refused = ask(
        &mut ws,
        serde_json::json!({ "type": "cue", "block": "nope", "word": 0 }),
    );
    assert_eq!(refused["type"], "error", "{refused}");
}

/// An edit made since in the author's editor is not undone over it.
#[test]
fn an_edit_is_not_undone_over_the_authors_own() {
    let served = voiced("prompt-voice-undo-theirs");
    let mut ws = session(served.addr);
    let reword = serde_json::json!({ "type": "reword", "line": "welcome", "text": "Hello there." });
    assert_eq!(ask(&mut ws, reword)["type"], "edited");
    let theirs = demo(&served).replace("Hello there.", "Hello from my editor.");
    std::fs::write(served.dir.join("scripts/demo.md"), &theirs).unwrap();
    let undone = ask(&mut ws, serde_json::json!({ "type": "undo_edit" }));
    assert_eq!(undone["type"], "error", "{undone}");
    assert_eq!(demo(&served), theirs);
}

/// A POST: the status code and each line of the body as it came.
fn post_lines(addr: SocketAddr, path: &str) -> (u16, Vec<serde_json::Value>) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
    write!(
        s,
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    s.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head[9..12].parse().unwrap();
    if status != 200 {
        return (status, Vec::new());
    }
    let body = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        dechunked(body)
    } else {
        body.to_string()
    };
    let lines = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l:?}")))
        .collect();
    (status, lines)
}

/// A chunked body, as a streamed response is sent, put back together.
fn dechunked(mut body: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = body.split_once("\r\n") {
        let size = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        out.push_str(&rest[..size]);
        body = rest[size..].trim_start_matches("\r\n");
    }
    out
}

/// Capturing from the prompter: `teleprompt capture`, its progress events
/// passed on line by line as it goes, and what it made last.
#[test]
fn the_prompter_captures_the_scripts_shots_and_says_how_it_goes() {
    let served = voiced("prompt-voice-make");
    let (status, events) = post_lines(served.addr, "/api/v1/make?job=capture");
    assert_eq!(status, 200, "{events:?}");
    let captured = events
        .iter()
        .filter(|e| e["event"] == "progress" && e["stage"] == "capture")
        .count();
    assert_eq!(captured, 2, "{events:?}");
    let last = events.last().unwrap();
    assert_eq!(
        last,
        &serde_json::json!({ "event": "made", "job": "capture", "video": null })
    );
    // The shots now have clips to show.
    let s = script(served.addr);
    assert!(s["shots"][0]["clip"].is_string(), "{s}");
    // Only a POST makes anything, and only a job it knows.
    assert_eq!(get(served.addr, "/api/v1/make?job=capture").0, 405);
    assert_eq!(post_lines(served.addr, "/api/v1/make?job=nope").0, 400);
}

/// The video as it will play: the manifest `dub` publishes, made by the
/// prompter on asking, with every line's audio synthesized first.
#[test]
fn the_manifest_is_the_one_dub_publishes() {
    let served = voiced("prompt-voice-manifest");
    let (status, body) = get(served.addr, "/api/v1/manifest");
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let served_manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let out = served.dir.join("out");
    let dubbed = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(&served.dir)
        .args(["dub", "scripts/demo.md", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        dubbed.status.success(),
        "{}",
        String::from_utf8_lossy(&dubbed.stderr)
    );
    let dubbed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("en/narration.json")).unwrap())
            .unwrap();
    assert_eq!(served_manifest, dubbed);
    // And the audio the page plays is the file `dub` writes.
    for line in dubbed["lines"].as_array().unwrap() {
        let id = line["id"].as_str().unwrap();
        let (status, wav) = get(served.addr, &format!("/api/v1/voice/{id}.wav?fit=1"));
        assert_eq!(status, 200, "{id}");
        let written = std::fs::read(out.join(format!("en/audio/{id}.wav"))).unwrap();
        assert!(
            wav == written,
            "line {id}: the page plays other audio than dub wrote"
        );
    }
}

/// With no narration there is no audio to read a format from, and the
/// manifest must still say what `dub` does (#24).
#[test]
fn a_script_with_no_narration_has_the_audio_format_dub_publishes() {
    let silent = "---\nteleprompt: 1\n---\n\n# Silence\n";
    let served = voiced_script("prompt-voice-silent", "silent.md", Some(silent));
    let manifest: serde_json::Value =
        serde_json::from_slice(&get(served.addr, "/api/v1/manifest").1).unwrap();
    let out = served.dir.join("out");
    let dubbed = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(&served.dir)
        .args(["dub", "scripts/silent.md", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        dubbed.status.success(),
        "{}",
        String::from_utf8_lossy(&dubbed.stderr)
    );
    let dubbed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("en/narration.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["audio"], dubbed["audio"]);
}

/// A `fit-line` line plays at the tempo it will be published at: asked for
/// as the video plays it, its audio is as long as the manifest says.
#[test]
fn a_fit_line_lines_audio_is_at_its_tempo_as_the_video_plays_it() {
    let script = "---\nteleprompt: 1\n---\n\n# Fit\n\n\
        Deployment is one command, and it streams progress as it goes. {#deploy}\n\n\
        ```teleprompt scene=mock policy=fit-line\nwait 1000ms\n```\n";
    let served = voiced_script("prompt-voice-fit", "fit.md", Some(script));
    let manifest: serde_json::Value =
        serde_json::from_slice(&get(served.addr, "/api/v1/manifest").1).unwrap();
    let line = &manifest["lines"][0];
    assert_eq!(line["tempo_permille"], 1150, "{manifest}");
    let (_, body) = get(served.addr, "/api/v1/voice/deploy.wav?fit=1");
    let pcm = teleprompt_plugin::voice::wav::decode(&body).unwrap();
    assert_eq!(pcm.duration_ms(), line["duration_ms"].as_u64().unwrap());
    // As the voice made it, otherwise.
    let (_, raw) = get(served.addr, "/api/v1/voice/deploy.wav");
    let raw = teleprompt_plugin::voice::wav::decode(&raw).unwrap();
    assert_ne!(raw.duration_ms(), pcm.duration_ms());
}

/// A save that no longer compiles is said beside the script as it last
/// compiled: the author sees the error, and keeps what they had.
#[test]
fn a_save_that_does_not_compile_is_said_beside_the_last_good_script() {
    let served = voiced("prompt-voice-broken");
    let manifest = get(served.addr, "/api/v1/manifest").1;
    let good = script(served.addr);
    assert!(good["error"].is_null(), "{good}");
    let md = served.dir.join("scripts/demo.md");
    let source = std::fs::read_to_string(&md).unwrap();
    std::fs::write(
        &md,
        format!("{source}\n```teleprompt scene=mock\nwiat 1s\n```\n"),
    )
    .unwrap();
    let broken = script(served.addr);
    let errors = broken["error"]
        .as_array()
        .unwrap_or_else(|| panic!("{broken}"));
    assert!(errors[0].as_str().unwrap().contains("wiat"), "{errors:?}");
    assert_eq!(broken["lines"], good["lines"]);
    // The video as it last compiled, still.
    assert_eq!(get(served.addr, "/api/v1/manifest").1, manifest);
    std::fs::write(&md, source).unwrap();
    assert!(script(served.addr)["error"].is_null());
}
