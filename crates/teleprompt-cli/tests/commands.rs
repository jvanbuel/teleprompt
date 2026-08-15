use std::path::{Path, PathBuf};
use std::process::Command;

use teleprompt_cli::cmd::{check::run_check, diff::run_diff, plan::run_plan};
use teleprompt_cli::project::Project;

const GOOD: &str = r#"---
scene: { mock: { adapter: mock } }
---

# Intro

One two three four five six. {#welcome}

```teleprompt scene=mock
wait 500ms
```
"#;

const BAD_ATTR: &str = "# Intro\n\nOne. {#a polcy=hold}\n";
const BAD_SCENE: &str = "# Intro\n\nOne. {#a}\n\n```teleprompt scene=mock\nclick things\n```\n";

fn project_with(script: &str) -> (Project, PathBuf) {
    let dir = tempdir();
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let path = dir.join("scripts/test.md");
    std::fs::write(&path, script).unwrap();
    (Project::discover(&dir).unwrap(), path)
}

#[test]
fn check_accepts_a_valid_script() {
    let (p, s) = project_with(GOOD);
    assert!(run_check(&p, &s, "en").is_ok());
}

#[test]
fn check_reports_an_unknown_attribute_key() {
    let (p, s) = project_with(BAD_ATTR);
    let errs = run_check(&p, &s, "en").unwrap_err();
    assert!(errs[0].contains("unknown attribute key `polcy`"));
}

#[test]
fn check_reports_adapter_validation_errors() {
    let (p, s) = project_with(BAD_SCENE);
    let errs = run_check(&p, &s, "en").unwrap_err();
    assert!(errs[0].contains("unknown mock directive"));
}

#[test]
fn check_does_not_write_anything() {
    let (p, s) = project_with(GOOD);
    let before = listing(&p.root);
    run_check(&p, &s, "en").unwrap();
    assert_eq!(listing(&p.root), before, "check must have no side effects");
}

#[test]
fn plan_produces_a_timeline_without_writing_one() {
    let (p, s) = project_with(GOOD);
    let out = run_plan(&p, &s, "en").unwrap();
    assert!(out.timeline.duration_ms > 0);
    assert!(!p.timeline_path("test.md", "en").exists());
}

#[test]
fn diff_against_a_missing_timeline_reports_every_beat_as_added() {
    let (p, s) = project_with(GOOD);
    let d = run_diff(&p, &s, "en").unwrap();
    assert_eq!(d.added.len(), 1);
    assert!(!d.is_empty());
}

#[test]
fn diff_against_an_identical_committed_timeline_is_empty() {
    let (p, s) = project_with(GOOD);
    let out = run_plan(&p, &s, "en").unwrap();
    let dest = p.timeline_path("test.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, serde_json::to_string_pretty(&out.timeline).unwrap()).unwrap();

    assert!(run_diff(&p, &s, "en").unwrap().is_empty());
}

#[test]
fn diff_detects_an_edited_paragraph() {
    let (p, s) = project_with(GOOD);
    let out = run_plan(&p, &s, "en").unwrap();
    let dest = p.timeline_path("test.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, serde_json::to_string_pretty(&out.timeline).unwrap()).unwrap();

    std::fs::write(
        &s,
        GOOD.replace("One two three four five six.", "One two three."),
    )
    .unwrap();
    let d = run_diff(&p, &s, "en").unwrap();
    assert_eq!(d.changed.len(), 1);
    assert!(d.shift_ms < 0, "a shorter paragraph shortens the video");
}

#[test]
fn a_malformed_timeline_on_disk_is_an_error_not_a_panic() {
    let (p, s) = project_with(GOOD);
    let dest = p.timeline_path("test.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, "{ not json").unwrap();
    assert!(run_diff(&p, &s, "en").is_err());
}

fn listing(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path.clone());
            }
            out.push(path.display().to_string());
        }
    }
    out.sort();
    out
}

fn tempdir() -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "teleprompt-cmd-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

/// Task 14 review round 1, finding 2: `plan` and `diff` must emit a typed,
/// parseable JSON payload on failure too, not just on success — a CI
/// consumer piping `--format json` at a failing command needs something it
/// can parse. These drive the actual compiled binary rather than the
/// library functions, since the bug was in `main.rs`'s wiring, not in
/// `run_plan`/`run_diff` themselves.
#[test]
fn plan_format_json_emits_a_typed_error_payload_on_failure() {
    let (p, _s) = project_with(BAD_ATTR);
    let bad = p.root.join("scripts/test.md");

    let output = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["--format", "json", "plan"])
        .arg(&bad)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("plan --format json on a failing script must print parseable JSON on stdout");
    assert_eq!(json["ok"], false);
    let errors = json["errors"].as_array().expect("errors must be an array");
    assert!(errors[0]
        .as_str()
        .unwrap()
        .contains("unknown attribute key `polcy`"));
}

#[test]
fn diff_format_json_emits_a_typed_error_payload_on_failure() {
    let (p, _s) = project_with(BAD_SCENE);
    let bad = p.root.join("scripts/test.md");

    let output = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["--format", "json", "diff"])
        .arg(&bad)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("diff --format json on a failing script must print parseable JSON on stdout");
    assert_eq!(json["ok"], false);
    let errors = json["errors"].as_array().expect("errors must be an array");
    assert!(errors[0]
        .as_str()
        .unwrap()
        .contains("unknown mock directive"));
}

/// Final review, item 3. `Path::new("demo.md").parent()` is `Some("")`, not
/// `None`, so the `unwrap_or(Path::new("."))` fallback never fired and
/// `"".canonicalize()` gave a bare `No such file or directory (os error 2)`
/// with no path in it. Every other test here, the README, and both manual
/// transcripts happen to pass a path with a directory component.
///
/// Driven through the binary with `current_dir` set, because the defect is
/// precisely about relative-path resolution against the process's working
/// directory; calling the command functions with a `Path` would not
/// reproduce it.
#[test]
fn every_script_command_works_on_a_bare_filename_from_the_scripts_directory() {
    let (p, _s) = project_with(GOOD);
    let scripts = p.root.join("scripts");

    for (args, what) in [
        (vec!["check"], "check"),
        (vec!["plan"], "plan"),
        (vec!["diff"], "diff"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
            .args(&args)
            .arg("test.md")
            .current_dir(&scripts)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "`teleprompt {what} test.md` from the script's own directory failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// The other half of item 3: when the path really is unreachable, the
/// diagnostic has to say which path. A bare ENOENT is unactionable.
#[test]
fn an_unreachable_script_path_is_named_in_the_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["check", "/nonexistent-teleprompt-dir/script.md"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("/nonexistent-teleprompt-dir"),
        "the error must name the path it could not reach: {stderr}"
    );
}
