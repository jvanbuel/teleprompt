use std::path::{Path, PathBuf};

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
