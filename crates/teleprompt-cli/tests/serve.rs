//! `serve`, driven the way a preview drives it.
//!
//! The server is bound on port 0 and handed to `serve_on`, so these run the
//! real accept loop, the real watcher and the real compile — the only thing
//! substituted is the port. Requests go over a plain socket rather than
//! through a client library, which keeps the test honest about the bytes the
//! server actually writes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use teleprompt_cli::cmd::serve::serve_on;
use teleprompt_cli::project::Project;

const SCRIPT: &str = "\
---
teleprompt: 1
scene:
  terminal:
    adapter: vhs
---

# Getting started

Welcome to teleprompt. This paragraph decides how long the tape below runs. {#welcome}

```teleprompt scene=terminal
Set TypingSpeed 40ms
Type \"teleprompt plan demo.md\"
Enter
Sleep 1s
```

The second paragraph, which the edit in these tests rewrites. {#second}

```teleprompt scene=terminal policy=concurrent
Type \"teleprompt diff demo.md\"
Enter
```
";

/// A scaffolded project with `SCRIPT` in it, served on a port the OS picks.
fn serving() -> (teleprompt_testkit::TestDir, PathBuf, SocketAddr) {
    let dir = teleprompt_testkit::test_dir("serve");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/preview.md");
    std::fs::write(&script, SCRIPT).unwrap();

    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    let addr = listener.local_addr().unwrap();

    let project = Project::discover(&dir).unwrap();
    let served = script.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = rt.block_on(serve_on(listener, &project, &served, "en"));
    });

    (dir, script, addr)
}

/// One GET, one response body. Blocking and connection-per-request, which is
/// exactly what the server answers.
fn get(addr: SocketAddr, path: &str) -> (String, Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match TcpStream::connect(addr) {
            Ok(mut s) => {
                write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
                let mut reader = BufReader::new(s);
                let mut status = String::new();
                reader.read_line(&mut status).unwrap();
                // Skip headers; the body is whatever follows the blank line.
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let mut body = Vec::new();
                reader.read_to_end(&mut body).unwrap();
                return (status.trim().to_string(), body);
            }
            Err(e) if Instant::now() < deadline => {
                // The first compile has to finish before anything is bound
                // to answer; a cold `null` synthesis is fast but not free.
                std::thread::sleep(Duration::from_millis(50));
                let _ = e;
            }
            Err(e) => panic!("server never answered on {addr}: {e}"),
        }
    }
}

fn json(addr: SocketAddr, path: &str) -> serde_json::Value {
    let (status, body) = get(addr, path);
    assert!(status.contains("200"), "{path}: {status}");
    serde_json::from_slice(&body).unwrap_or_else(|e| panic!("{path} is not JSON: {e}"))
}

/// Polls until the server publishes a generation past `after`, or gives up.
fn await_generation(addr: SocketAddr, after: u64) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = json(addr, "/state.json");
        if state["generation"].as_u64().unwrap() > after {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "the watcher never noticed the edit"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn the_preview_is_served_a_manifest_with_shots_in_it() {
    let (_dir, _script, addr) = serving();
    let m = json(addr, "/manifest.json");

    // The page refuses any manifest but the version it was written for, so
    // that version must be the one the server sends.
    let (_, page) = get(addr, "/");
    let page = String::from_utf8(page).unwrap();
    let marker = "manifest.manifest_version !== ";
    let at = page.find(marker).expect("the page checks the version") + marker.len();
    let accepted: u64 = page[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap();
    assert_eq!(
        m["manifest_version"], accepted,
        "the preview would refuse it"
    );
    assert_eq!(m["lines"].as_array().unwrap().len(), 2);
    let shots = m["shots"].as_array().unwrap();
    assert_eq!(shots.len(), 2, "one shot per action block");
    assert_eq!(shots[0]["scene"], "terminal");
    assert_eq!(shots[0]["policy"], "hold");
    assert_eq!(shots[1]["policy"], "concurrent");
}

#[test]
fn a_shot_source_is_served_so_the_scene_can_be_drawn() {
    let (_dir, _script, addr) = serving();
    let m = json(addr, "/manifest.json");
    let shot = m["shots"][0]["shot"].as_str().unwrap().to_string();

    // A shot id carries a `#`, which a client must percent-encode or the
    // fragment split eats the rest of the path.
    let (status, body) = get(addr, &format!("/shots/{}", shot.replace('#', "%23")));
    assert!(status.contains("200"), "{status}");
    let source = String::from_utf8(body).unwrap();
    assert!(source.contains("teleprompt plan demo.md"), "{source}");
}

#[test]
fn a_lines_audio_comes_back_as_a_wav() {
    let (_dir, _script, addr) = serving();
    let (status, body) = get(addr, "/audio/welcome.wav");
    assert!(status.contains("200"), "{status}");
    assert_eq!(&body[0..4], b"RIFF", "the cache stores WAV, so serve it");
}

#[test]
fn nothing_outside_the_cache_can_be_asked_for() {
    let (_dir, _script, addr) = serving();
    for path in [
        "/audio/..%2f..%2fetc%2fpasswd.wav",
        "/shots/..%2f..%2fsecrets",
        "/../Cargo.toml",
    ] {
        let (status, _) = get(addr, path);
        assert!(
            status.contains("404") || status.contains("400"),
            "{path} answered {status}"
        );
    }
}

#[test]
fn editing_a_paragraph_republishes_and_names_what_moved() {
    let (_dir, script, addr) = serving();
    let before = json(addr, "/state.json");
    assert_eq!(before["generation"], 1);
    assert!(before["changed"].as_array().unwrap().is_empty());
    let was = before["duration_ms"].as_u64().unwrap();

    // The second paragraph gets longer, which moves its own item and
    // everything scheduled after it.
    let src = std::fs::read_to_string(&script).unwrap();
    std::fs::write(
        &script,
        src.replace(
            "The second paragraph, which the edit in these tests rewrites.",
            "The second paragraph, rewritten at some considerable length so that \
             the narration takes materially longer to speak than it did before, \
             which is the whole point of watching what an edit costs.",
        ),
    )
    .unwrap();

    let after = await_generation(addr, 1);
    let changed: Vec<&str> = after["changed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();

    assert!(
        changed.contains(&"second"),
        "the edited line should be named: {changed:?}"
    );
    assert!(
        after["duration_ms"].as_u64().unwrap() > was,
        "a longer paragraph makes a longer video"
    );

    // And the manifest the preview reads is the new one.
    let m = json(addr, "/manifest.json");
    let second = m["lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "second")
        .unwrap()
        .clone();
    assert!(second["text"]
        .as_str()
        .unwrap()
        .contains("considerable length"));
}

#[test]
fn a_script_that_stops_compiling_keeps_the_last_good_preview() {
    let (_dir, script, addr) = serving();
    let good = json(addr, "/manifest.json");

    std::fs::write(
        &script,
        format!("{SCRIPT}\n```teleprompt scene=terminal\nTyp \"oops\"\n```\n"),
    )
    .unwrap();

    let state = await_generation(addr, 1);
    let errors = state["error"].as_array().expect("the failure is reported");
    assert!(
        errors[0].as_str().unwrap().contains("Typ"),
        "the error names the bad line: {errors:?}"
    );

    // A blank preview would answer a question nobody asked. What the author
    // wants is the error plus the video they had a moment ago.
    let still = json(addr, "/manifest.json");
    assert_eq!(still, good, "the last compiling manifest is still served");
}

#[test]
fn what_sits_before_an_edit_is_not_reported_as_moved() {
    // The first preview must publish the durations it will still be
    // publishing after the next save. If generation 1 carries estimates and
    // generation 2 carries measurements, every line "moves" on the first
    // edit and the preview has nowhere meaningful to jump to.
    let (_dir, script, addr) = serving();
    let first = json(addr, "/manifest.json");
    let sources: Vec<&str> = first["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["duration_source"].as_str().unwrap())
        .collect();
    assert!(
        sources.iter().all(|s| *s == "measured"),
        "serve synthesizes before it publishes, so nothing is an estimate: {sources:?}"
    );

    let src = std::fs::read_to_string(&script).unwrap();
    std::fs::write(
        &script,
        src.replace(
            "The second paragraph, which the edit in these tests rewrites.",
            "The second paragraph, rewritten at considerable length so that the \
             narration takes materially longer to speak than it did before.",
        ),
    )
    .unwrap();

    let changed = await_generation(addr, 1);
    let changed: Vec<&str> = changed["changed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        !changed.contains(&"welcome"),
        "the paragraph above the edit did not move: {changed:?}"
    );
}

/// A save the clock cannot see.
///
/// The watcher compared modification times, so an edit landing inside the
/// filesystem's timestamp resolution was invisible — not late, invisible,
/// because the next comparison is against the same value. CI caught it as
/// "the watcher never noticed the edit" on a run that had nothing to do
/// with watching; this reproduces it deterministically by putting the
/// timestamp back afterwards.
#[test]
fn an_edit_that_does_not_move_the_clock_is_still_noticed() {
    let (_dir, script, addr) = serving();
    let before = json(addr, "/state.json");
    assert_eq!(before["generation"], 1);

    let was = std::fs::metadata(&script).unwrap();
    let src = std::fs::read_to_string(&script).unwrap();
    std::fs::write(
        &script,
        src.replace(
            "The second paragraph, which the edit in these tests rewrites.",
            "The second paragraph, rewritten without the clock noticing at all.",
        ),
    )
    .unwrap();

    // Put the timestamps back exactly where they were, which is what a
    // coarse filesystem does for free.
    let restored = std::fs::File::options().write(true).open(&script).unwrap();
    restored
        .set_times(
            std::fs::FileTimes::new()
                .set_accessed(was.accessed().unwrap())
                .set_modified(was.modified().unwrap()),
        )
        .unwrap();

    let after = await_generation(addr, 1);
    let changed: Vec<&str> = after["changed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(changed.contains(&"second"), "{changed:?}");
}
