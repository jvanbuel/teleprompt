//! `record`'s tools, run as an author runs them: in a terminal, typed into
//! with pauses, or in a window, and stopped as the app stops them. (The
//! microphone needs a microphone; `record_listen.rs` plays it one.)
//!
//! A recorder takes over the terminal it runs in, so each test runs again
//! as a child in a pseudo-terminal of its own, and the child records. A
//! tool that is not installed skips its test unless
//! `TELEPROMPT_REQUIRE_TOOLS` is set.
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use teleprompt_plugin::record::Start;
use teleprompt_registry::Recording as Recorder;

/// Set in the child: where it writes what it recorded.
const CHILD: &str = "TELEPROMPT_RECORD_TEST_OUT";

fn recorder(plugin: &str) -> Option<Recorder> {
    let r = teleprompt_registry::registry()
        .recorders()
        .into_iter()
        .find(|r| r.plugin == plugin)
        .expect("a recorder");
    match r.unavailable() {
        None => Some(r),
        Some(why) if std::env::var_os("TELEPROMPT_REQUIRE_TOOLS").is_some() => {
            panic!("{plugin}: {why}")
        }
        Some(_) => None,
    }
}

/// In the child, records with `scene plugin` until the tool ends or the parent
/// asks it to stop, and writes each step's start and the whole file.
fn child(plugin: &str, url: Option<&str>) -> bool {
    let Some(out) = std::env::var_os(CHILD).map(PathBuf::from) else {
        return false;
    };
    let recorder = recorder(plugin).expect("checked by the parent");
    let stop = Arc::new(AtomicBool::new(false));
    let asked = out.with_extension("stop");
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || loop {
        if asked.exists() {
            flag.store(true, Ordering::SeqCst);
        }
        std::thread::sleep(Duration::from_millis(50));
    });
    let dir = out.parent().unwrap();
    let file = dir.join(format!("session.{}", recorder.extension()));
    let shell = vec!["sh".to_string()];
    let recorded = recorder
        .start(
            &file,
            &Start {
                cwd: dir,
                shell: &shell,
                url,
            },
        )
        .and_then(|r| r.wait(&stop))
        .unwrap_or_else(|e| panic!("{e}"));
    let starts: Vec<String> = recorded
        .steps
        .iter()
        .map(|s| s.start_ms.to_string())
        .collect();
    let cuts: Vec<usize> = (1..recorded.steps.len()).collect();
    std::fs::write(
        &out,
        format!("{}\n{}", starts.join(" "), recorded.marked(&cuts)),
    )
    .unwrap();
    true
}

/// What the child recorded: each step's start, and the file cut between
/// every step.
struct Recording {
    starts: Vec<u64>,
    marked: String,
}

/// Runs `test` again as a child in a terminal, types `keys` into it (each
/// after its pause), then, if `stop`, asks it to stop as the app would.
fn run_child(test: &str, dir: &Path, keys: &[(u64, &str)], stop: bool) -> Recording {
    let out = dir.join("recorded.txt");
    let pty = native_pty_system()
        .openpty(PtySize {
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
    command.args([test, "--exact", "--nocapture", "--test-threads", "1"]);
    command.env(CHILD, &out);
    command.cwd(dir);
    let mut child = pty.slave.spawn_command(command).unwrap();
    drop(pty.slave);
    let mut reader = pty.master.try_clone_reader().unwrap();
    let screen = Arc::new(std::sync::Mutex::new(String::new()));
    let seen = Arc::clone(&screen);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            seen.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    let mut writer = pty.master.take_writer().unwrap();
    for (pause, text) in keys {
        std::thread::sleep(Duration::from_millis(*pause));
        writer.write_all(text.as_bytes()).unwrap();
        writer.flush().unwrap();
    }
    if stop {
        std::fs::write(out.with_extension("stop"), "").unwrap();
    }
    let began = Instant::now();
    while child.try_wait().unwrap().is_none() {
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "the recording did not end:\n{}",
            screen.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let text = std::fs::read_to_string(&out)
        .unwrap_or_else(|_| panic!("the child wrote nothing:\n{}", screen.lock().unwrap()));
    let (starts, marked) = text.split_once('\n').unwrap();
    Recording {
        starts: starts
            .split_whitespace()
            .map(|s| s.parse().unwrap())
            .collect(),
        marked: marked.to_string(),
    }
}

/// `echo one`, a pause, `echo two`, then `exit`.
const SESSION: &[(u64, &str)] = &[(1500, "echo one\r"), (1500, "echo two\r"), (1000, "exit\r")];

#[test]
fn asciinema_records_each_command_and_marks_between_them() {
    if child("asciinema", None) || recorder("asciinema").is_none() {
        return;
    }
    let dir = teleprompt_testkit::test_dir("record-asciinema");
    let r = run_child(
        "asciinema_records_each_command_and_marks_between_them",
        &dir,
        SESSION,
        false,
    );
    assert_eq!(r.starts.len(), 2, "{}", r.marked);
    let gap = r.starts[1] - r.starts[0];
    assert!((1300..2500).contains(&gap), "{:?}", r.starts);

    use teleprompt_asciinema::scene::{parse, select};
    let cast = parse(&r.marked).unwrap_or_else(|e| panic!("{e:?}\n{}", r.marked));
    let second: String = select(&cast, "2")
        .unwrap()
        .events
        .iter()
        .map(|e| e.data.clone())
        .collect();
    assert!(
        second.contains("two") && !second.contains("one"),
        "{second}"
    );
    assert!(!r.marked.contains("exit"), "{}", r.marked);
}

/// The app's Stop: the shell is hung up and what was recorded is kept.
#[test]
fn asciinema_stopped_from_outside_keeps_what_it_recorded() {
    if child("asciinema", None) || recorder("asciinema").is_none() {
        return;
    }
    let dir = teleprompt_testkit::test_dir("record-asciinema-stop");
    let r = run_child(
        "asciinema_stopped_from_outside_keeps_what_it_recorded",
        &dir,
        &[(1500, "echo kept\r"), (1000, "")],
        true,
    );
    assert_eq!(r.starts.len(), 1, "{}", r.marked);
    assert!(r.marked.contains("kept"), "{}", r.marked);
}

#[test]
fn vhs_records_a_tape_of_each_command() {
    if child("vhs", None) || recorder("vhs").is_none() {
        return;
    }
    let dir = teleprompt_testkit::test_dir("record-vhs");
    let r = run_child("vhs_records_a_tape_of_each_command", &dir, SESSION, false);
    assert!(
        r.marked.contains("Type \"echo one\"\nEnter\n")
            && r.marked.contains("# mark\nType \"echo two\"\nEnter\n"),
        "{}",
        r.marked
    );
    assert_eq!(r.starts.len(), 2, "{}", r.marked);
    assert!(r.starts[1] > r.starts[0] + 1000, "{:?}", r.starts);
}

/// `vhs record` stopped from outside writes its tape all the same.
#[test]
fn vhs_stopped_from_outside_keeps_its_tape() {
    if child("vhs", None) || recorder("vhs").is_none() {
        return;
    }
    let dir = teleprompt_testkit::test_dir("record-vhs-stop");
    let r = run_child(
        "vhs_stopped_from_outside_keeps_its_tape",
        &dir,
        &[(1500, "echo kept\r"), (1000, "")],
        true,
    );
    assert!(r.marked.contains("Type \"echo kept\""), "{}", r.marked);
}

/// codegen in its own window, on a display: the page it opens is its first
/// step, and a stop ends it with the script kept.
#[test]
fn playwright_codegen_records_the_page_it_opened() {
    let page = "data:text/html,<button>Deploy</button>";
    if child("playwright", Some(page)) || recorder("playwright").is_none() {
        return;
    }
    if std::env::var_os("DISPLAY").is_none() {
        return;
    }
    let dir = teleprompt_testkit::test_dir("record-playwright");
    let r = run_child(
        "playwright_codegen_records_the_page_it_opened",
        &dir,
        &[(8000, "")],
        true,
    );
    assert!(r.marked.contains("await page.goto("), "{}", r.marked);
}

/// What an app offers: every tool, and whether it can record here.
#[test]
fn record_lists_its_tools() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["--format", "json", "record", "--tools"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tools: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["plugin"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["asciinema", "vhs", "playwright"]);
    assert_eq!(tools[2]["in_terminal"], false);
}

/// What the app reads, since it cannot wait on a terminal it did not
/// start itself: a recording that cannot start says so in its status file.
#[test]
fn a_recording_that_cannot_start_says_why_in_its_status() {
    let dir = teleprompt_testkit::test_dir("record-status");
    teleprompt_project::new::scaffold(&dir).unwrap();
    let status = dir.join("status.json");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(dir.path())
        .args([
            "record",
            "scripts/session.md",
            "--model",
            "no-such-model",
            "--status",
        ])
        .arg(&status)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&status).unwrap()).unwrap();
    assert_eq!(status["state"], "failed", "{status}");
    assert!(status["error"].as_str().unwrap().len() > 10, "{status}");
}

/// A tool this build cannot record with is named, with those it can.
#[test]
fn an_unknown_tool_names_the_ones_that_record() {
    let dir = teleprompt_testkit::test_dir("record-unknown");
    teleprompt_project::new::scaffold(&dir).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(dir.path())
        .args(["record", "scripts/s.md", "--model", ".", "--with", "slidev"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("asciinema, vhs, playwright"), "{err}");
}
