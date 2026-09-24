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
fn diff_against_a_missing_timeline_reports_every_item_as_added() {
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

/// `plan` and `diff` must emit a typed, parseable JSON payload on failure too,
/// not just on success — a CI consumer piping `--format json` at a failing
/// command needs something it can parse. These drive the actual compiled binary
/// rather than the library functions, since the bug was in `main.rs`'s wiring,
/// not in `run_plan`/`run_diff` themselves.
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

/// `Path::new("demo.md").parent()` is `Some("")`, not `None`, so the
/// `unwrap_or(Path::new("."))` fallback never fired and `"".canonicalize()`
/// gave a bare `No such file or directory (os error 2)` with no path in it.
/// Every other test here, the README, and both manual transcripts happen to
/// pass a path with a directory component.
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

#[test]
fn an_unknown_voice_backend_is_a_validation_error_naming_what_exists() {
    let (p, s) =
        project_with("---\nvoice: { backend: nope }\n---\n\n# Intro\n\nOne two three. {#a}\n");
    let errors = run_check(&p, &s, "en").expect_err("unknown backend must fail check");
    let joined = errors.join("\n");
    assert!(joined.contains("nope"), "{joined}");
    assert!(
        joined.contains("null"),
        "must name what is available: {joined}"
    );
}

#[test]
fn the_default_backend_is_null_so_existing_scripts_keep_working() {
    let (p, s) = project_with("# Intro\n\nOne two three. {#a}\n");
    assert!(run_check(&p, &s, "en").is_ok());
}

/// The extra requirement beyond the brief: a line-level `voice.backend=`
/// that disagrees with the program's resolved backend must fail `check`
/// rather than being silently ignored — `VoiceContext` carries exactly one
/// backend per compile, so an ignored override would let two lines that
/// differ only by backend collide on one cache key and one would be served
/// the other's audio.
#[test]
fn a_line_level_backend_override_that_disagrees_with_the_resolved_backend_is_rejected() {
    let (p, s) = project_with("# Intro\n\nOne two three. {#a voice.backend=nope}\n");
    let errors = run_check(&p, &s, "en").expect_err("a disagreeing per-line backend must fail");
    let joined = errors.join("\n");
    assert!(joined.contains("line `a`"), "must name the line: {joined}");
    assert!(
        joined.contains("nope") && joined.contains("null"),
        "must name both the line's requested backend and the resolved one: {joined}"
    );
    assert!(
        joined.contains("not supported yet"),
        "must say per-line backends are not supported yet: {joined}"
    );
}

/// A line-level `voice.backend=` that agrees with the resolved backend
/// is not an override at all and must not be rejected.
#[test]
fn a_line_level_backend_that_matches_the_resolved_backend_is_fine() {
    let (p, s) = project_with("# Intro\n\nOne two three. {#a voice.backend=null}\n");
    assert!(run_check(&p, &s, "en").is_ok());
}

/// A chapter-level `voice.backend` override with no line attribute must still
/// be rejected (Delivery A supports one backend per compile, full stop), but
/// `Element::Narration`'s config is already merged and cannot say which layer
/// produced the value. The diagnostic must therefore describe the effect
/// ("resolves to") rather than accuse the line of writing an attribute it never
/// wrote, and the help text must mention that chapter-level overrides are
/// unsupported too.
#[test]
fn a_chapter_level_backend_override_is_reported_without_claiming_the_line_set_it() {
    let (p, s) = project_with(
        "# Intro\n\n```yaml teleprompt\nvoice:\n  backend: elsewhere\n```\n\nOne two three. {#a}\n",
    );
    let errors = run_check(&p, &s, "en").expect_err("a chapter-level backend override must fail");
    let joined = errors.join("\n");
    assert!(
        !joined.contains("sets"),
        "must not claim the line wrote an attribute it did not: {joined}"
    );
    assert!(
        joined.contains("resolves to voice backend `elsewhere`"),
        "must describe the effect, not a guessed cause: {joined}"
    );
    assert!(
        joined.contains("chapter"),
        "help must mention chapter-level overrides are unsupported too: {joined}"
    );
}

/// A chapter-wide override affecting several lines must produce exactly one
/// diagnostic naming all of them, not one per line.
#[test]
fn a_chapter_level_backend_override_across_several_lines_is_one_diagnostic() {
    let (p, s) = project_with(
        "# Intro\n\n```yaml teleprompt\nvoice:\n  backend: elsewhere\n```\n\n\
         One. {#a}\n\nTwo. {#b}\n\nThree. {#c}\n",
    );
    let errors = run_check(&p, &s, "en").expect_err("a chapter-level backend override must fail");
    assert_eq!(
        errors.len(),
        1,
        "one diagnostic per offending value, not one per line: {errors:?}"
    );
    let joined = errors.join("\n");
    for id in ["a", "b", "c"] {
        assert!(
            joined.contains(&format!("`{id}`")),
            "must name line `{id}`: {joined}"
        );
    }
}

/// C1, through the real binary. The panic was the point: exit 101 is not a
/// code `teleprompt_cli::output::exit_code_for` can issue, so the process
/// bypassed the CLI's whole error contract. Driven through the binary rather
/// than `run_check` because a panic in a library call would abort the test
/// harness instead of being observed as an exit code.
#[test]
fn check_on_a_zero_voice_speed_exits_two_rather_than_panicking() {
    let (p, _s) = project_with("---\nvoice: { speed: 0 }\n---\n\n# Intro\n\nOne two. {#a}\n");

    let output = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .arg("check")
        .arg(p.root.join("scripts/test.md"))
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(2),
        "expected a validation error, got {:?}: {stderr}",
        output.status.code()
    );
    assert!(
        stderr.contains("speed must be greater than zero"),
        "{stderr}"
    );
}

/// The row that matters most in C1's table: before this, `check` exited 0 on
/// a script `dub` exited 1 on. A validate-only command that passes what the
/// real command refuses is the exact failure this delivery exists to
/// prevent, so assert the two agree rather than asserting either alone.
#[test]
fn check_and_dub_agree_about_a_negative_voice_speed() {
    let (p, _s) = project_with("---\nvoice: { speed: -1 }\n---\n\n# Intro\n\nOne two. {#a}\n");
    let script = p.root.join("scripts/test.md");
    let out_dir = p.root.join("out");

    let check = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .arg("check")
        .arg(&script)
        .output()
        .unwrap();
    let dub = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["dub"])
        .arg(&script)
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();

    assert_eq!(check.status.code(), Some(2), "check must reject");
    assert_eq!(
        dub.status.code(),
        check.status.code(),
        "dub must reject exactly what check does"
    );
}

/// `doctor`'s cache root was CWD-relative, so from any subdirectory of a
/// project with a full cache it reported `0 entries` — worse than reporting
/// nothing, because it looks like an answer. `check` and `dub` root theirs
/// at the project; so does this now, and the report says which root it used.
#[test]
fn doctor_reports_the_projects_cache_from_a_subdirectory() {
    let (p, _s) = project_with(GOOD);
    let dubbed = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["dub", "scripts/test.md", "--out", "out"])
        .current_dir(&p.root)
        .output()
        .unwrap();
    assert!(
        dubbed.status.success(),
        "{}",
        String::from_utf8_lossy(&dubbed.stderr)
    );

    let out = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["--format", "json", "doctor"])
        .current_dir(p.root.join("scripts"))
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    assert!(
        json["cache_entries"].as_u64().unwrap() > 0,
        "doctor must see the project's cache from inside it: {json}"
    );
    assert!(
        json["cache_root"].as_str().unwrap().contains(".teleprompt"),
        "the report must say which root it counted: {json}"
    );
}
