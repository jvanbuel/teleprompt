//! `--format json` progress, for an app to show: one JSON event per line
//! on stderr as each stage goes, and the report on stdout at the end.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// The command's stderr, every line of it an event, and its report.
fn run(root: &Path, args: &[&str]) -> (Vec<Value>, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(root)
        .args(["--format", "json"])
        .args(args)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    let events = stderr
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|_| panic!("not an event: {l}")))
        .collect();
    let report = serde_json::from_slice(&out.stdout).unwrap();
    (events, report)
}

fn stage<'a>(events: &'a [Value], name: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|e| e["event"] == "progress" && e["stage"] == name)
        .collect()
}

#[test]
fn capture_says_each_shot_it_records() {
    let dir = teleprompt_testkit::test_dir("progress-capture");
    teleprompt_project::new::scaffold(&dir).unwrap();
    let (events, report) = run(&dir, &["capture", "scripts/demo.md"]);
    let shots = stage(&events, "capture");
    assert_eq!(shots.len(), 2, "{events:?}");
    assert_eq!(shots[0]["done"], 1);
    assert_eq!(shots[1]["done"], shots[1]["of"]);
    assert_eq!(shots[0]["shot"], "welcome-a#0");
    let voice = stage(&events, "voice");
    assert_eq!(voice.last().unwrap()["done"], 2, "{events:?}");
    assert!(report.is_object());
}

#[test]
fn build_says_how_far_the_render_has_got() {
    let dir = teleprompt_testkit::test_dir("progress-build");
    teleprompt_project::new::scaffold(&dir).unwrap();
    let (events, report) = run(&dir, &["build", "scripts/demo.md", "--out", "out.mp4"]);
    let render = stage(&events, "render");
    assert!(!render.is_empty(), "{events:?}");
    let last = render.last().unwrap();
    assert_eq!(last["done_ms"], last["of_ms"], "{events:?}");
    assert!(report.is_object());
}
