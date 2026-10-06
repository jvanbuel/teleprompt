use std::path::Path;
use std::process::{Command, Output};

use teleprompt_project::project::Project;

const SCRIPT: &str = "\
# Quick start

Every video in this repository is built from a script you can read.
";

fn tempdir(tag: &str) -> teleprompt_testkit::TestDir {
    teleprompt_testkit::test_dir(&format!("voiceloop-{tag}"))
}

/// Scaffold a project and drop `script` at `scripts/test.md`, mirroring
/// `commands.rs::project_with`. Returns the project root.
fn project_with(tag: &str, script: &str) -> teleprompt_testkit::TestDir {
    let dir = tempdir(tag);
    teleprompt_project::new::scaffold(&dir).unwrap();
    std::fs::write(dir.join("scripts/test.md"), script).unwrap();
    Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    dir
}

fn tp(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("process exited normally")
}

/// The whole point of Delivery A, end to end: plan is estimated and instant,
/// dub measures it, and plan afterwards is measured without becoming slow.
#[test]
fn plan_is_estimated_until_dub_measures_it() {
    let root = project_with("loop", SCRIPT);

    let first = tp(&root, &["--format", "json", "plan", "scripts/test.md"]);
    assert_eq!(code(&first), 0);
    let t: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(t["entries"][0]["narration"]["duration_source"], "estimated");

    assert_eq!(
        code(&tp(&root, &["dub", "scripts/test.md", "--out", "out"])),
        0
    );

    let second = tp(&root, &["--format", "json", "plan", "scripts/test.md"]);
    let t2: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(t2["entries"][0]["narration"]["duration_source"], "measured");
}

#[test]
fn plan_warns_when_it_emits_estimated_durations() {
    let root = project_with("warn", SCRIPT);
    let out = tp(&root, &["plan", "scripts/test.md"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("estimated"),
        "a timeline committed from a cold cache drifts on the next dub: {stderr}"
    );
}

#[test]
fn the_cache_survives_across_runs_so_the_second_dub_is_a_no_op_diff() {
    let root = project_with("stable", SCRIPT);
    tp(&root, &["dub", "scripts/test.md", "--out", "out"]);
    let a = std::fs::read(root.join("out/en/narration.json")).unwrap();
    tp(&root, &["dub", "scripts/test.md", "--out", "out"]);
    let b = std::fs::read(root.join("out/en/narration.json")).unwrap();
    assert_eq!(a, b, "a warm cache must reproduce the manifest exactly");
}
