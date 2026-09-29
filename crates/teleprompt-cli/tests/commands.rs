use std::path::{Path, PathBuf};
use std::process::Command;
use teleprompt_core::SpanMs;

use teleprompt_cli::cmd::{check::run_check, plan::run_plan, plan::run_plan_check};
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

fn project_with(script: &str) -> (teleprompt_testkit::TestDir, Project, PathBuf) {
    let dir = tempdir();
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let path = dir.join("scripts/test.md");
    std::fs::write(&path, script).unwrap();
    let project = Project::discover(&dir).unwrap();
    (dir, project, path)
}

#[test]
fn check_accepts_a_valid_script() {
    let (_dir, p, s) = project_with(GOOD);
    assert!(run_check(&p, &s, "en").is_ok());
}

#[test]
fn check_reports_an_unknown_attribute_key() {
    let (_dir, p, s) = project_with(BAD_ATTR);
    let errs = run_check(&p, &s, "en").unwrap_err();
    assert!(errs[0].contains("unknown attribute key `polcy`"));
}

#[test]
fn check_reports_adapter_validation_errors() {
    let (_dir, p, s) = project_with(BAD_SCENE);
    let errs = run_check(&p, &s, "en").unwrap_err();
    assert!(errs[0].contains("unknown mock directive"));
}

#[test]
fn check_does_not_write_anything() {
    let (_dir, p, s) = project_with(GOOD);
    let before = listing(&p.root);
    run_check(&p, &s, "en").unwrap();
    assert_eq!(listing(&p.root), before, "check must have no side effects");
}

#[test]
fn plan_produces_a_timeline_without_writing_one() {
    let (_dir, p, s) = project_with(GOOD);
    let out = run_plan(&p, &s, "en").unwrap();
    assert!(out.timeline.duration_ms > SpanMs::of(0));
    assert!(!p.timeline_path("test.md", "en").exists());
}

#[test]
fn diff_against_a_missing_timeline_reports_every_item_as_added() {
    let (_dir, p, s) = project_with(GOOD);
    let d = run_plan_check(&p, &s, "en").unwrap();
    assert_eq!(d.added.len(), 1);
    assert!(!d.is_empty());
}

#[test]
fn diff_against_an_identical_committed_timeline_is_empty() {
    let (_dir, p, s) = project_with(GOOD);
    let out = run_plan(&p, &s, "en").unwrap();
    let dest = p.timeline_path("test.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, serde_json::to_string_pretty(&out.timeline).unwrap()).unwrap();

    assert!(run_plan_check(&p, &s, "en").unwrap().is_empty());
}

#[test]
fn diff_detects_an_edited_paragraph() {
    let (_dir, p, s) = project_with(GOOD);
    let out = run_plan(&p, &s, "en").unwrap();
    let dest = p.timeline_path("test.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, serde_json::to_string_pretty(&out.timeline).unwrap()).unwrap();

    std::fs::write(
        &s,
        GOOD.replace("One two three four five six.", "One two three."),
    )
    .unwrap();
    let d = run_plan_check(&p, &s, "en").unwrap();
    assert_eq!(d.changed.len(), 1);
    assert!(d.shift_ms < 0, "a shorter paragraph shortens the video");
}

#[test]
fn a_malformed_timeline_on_disk_is_an_error_not_a_panic() {
    let (_dir, p, s) = project_with(GOOD);
    let dest = p.timeline_path("test.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, "{ not json").unwrap();
    assert!(run_plan_check(&p, &s, "en").is_err());
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

fn tempdir() -> teleprompt_testkit::TestDir {
    teleprompt_testkit::test_dir("cmd")
}

/// `plan` and `plan --check` must emit a typed, parseable JSON payload on failure too,
/// not just on success — a CI consumer piping `--format json` at a failing
/// command needs something it can parse. These drive the actual compiled binary
/// rather than the library functions, since the bug was in `main.rs`'s wiring,
/// not in `run_plan`/`run_plan_check` themselves.
#[test]
fn plan_format_json_emits_a_typed_error_payload_on_failure() {
    let (_dir, p, _s) = project_with(BAD_ATTR);
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
fn plan_check_format_json_emits_a_typed_error_payload_on_failure() {
    let (_dir, p, _s) = project_with(BAD_SCENE);
    let bad = p.root.join("scripts/test.md");

    let output = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["--format", "json", "plan", "--check"])
        .arg(&bad)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect(
        "plan --check --format json on a failing script must print parseable JSON on stdout",
    );
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
    let (_dir, p, _s) = project_with(GOOD);
    let scripts = p.root.join("scripts");

    for (args, what) in [
        (vec!["check"], "check"),
        (vec!["plan"], "plan"),
        (vec!["plan", "--check"], "plan --check"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
            .args(&args)
            .arg("test.md")
            .current_dir(&scripts)
            .output()
            .unwrap();
        // Nothing is committed, so `plan --check` reports drift: 3.
        assert!(
            matches!(output.status.code(), Some(0 | 3)),
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
    let (_dir, p, s) =
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
    let (_dir, p, s) = project_with("# Intro\n\nOne two three. {#a}\n");
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
    let (_dir, p, s) = project_with("# Intro\n\nOne two three. {#a voice.backend=nope}\n");
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
    let (_dir, p, s) = project_with("# Intro\n\nOne two three. {#a voice.backend=null}\n");
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
    let (_dir, p, s) = project_with(
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
    let (_dir, p, s) = project_with(
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
    let (_dir, p, _s) = project_with("---\nvoice: { speed: 0 }\n---\n\n# Intro\n\nOne two. {#a}\n");

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
    let (_dir, p, _s) =
        project_with("---\nvoice: { speed: -1 }\n---\n\n# Intro\n\nOne two. {#a}\n");
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
    let (_dir, p, _s) = project_with(GOOD);
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

/// A duration no one means is a validation error naming the attribute, not
/// an overflow panic in the scheduler (#22).
#[test]
fn an_absurd_lead_in_is_a_validation_error_not_a_panic() {
    let (_dir, p, s) = project_with(
        "---\nteleprompt: 1\n---\n\n# One\n\n\
         First line. {#a lead_in=18446744073709551615ms}\n\nSecond. {#b}\n",
    );
    let errors = run_check(&p, &s, "en").expect_err("an absurd lead_in must fail check");
    let joined = errors.join("\n");
    assert!(joined.contains("lead_in"), "{joined}");
}

/// `edit` is how the apps write a drag into the script: callable, but not
/// among the commands a person is shown.
#[test]
fn edit_runs_but_is_not_listed() {
    let help = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .arg("--help")
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&help.stdout);
    assert!(listed.contains("check"), "{listed}");
    assert!(!listed.contains("\n  edit "), "{listed}");
    let own = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["edit", "--help"])
        .output()
        .unwrap();
    assert!(own.status.success());
}

/// `plan --check` is the drift gate: exit 3 while the committed timeline
/// differs from the script's, 0 once the plan is committed.
#[test]
fn plan_check_exits_3_on_drift_and_0_once_committed() {
    let (_dir, p, script) = project_with(GOOD);
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_teleprompt"))
            .args(["--format", "json"])
            .args(args)
            .arg(&script)
            .output()
            .unwrap()
    };
    assert_eq!(run(&["plan", "--check"]).status.code(), Some(3));
    let plan = run(&["plan"]);
    assert!(plan.status.success());
    let committed = p.root.join("timelines/test.en.json");
    std::fs::create_dir_all(committed.parent().unwrap()).unwrap();
    std::fs::write(&committed, &plan.stdout).unwrap();
    let check = run(&["plan", "--check"]);
    assert_eq!(
        check.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
}

/// The commands `plan --check` and `prompt --preview` replaced say so,
/// whatever they are given, rather than being unknown.
#[test]
fn a_removed_command_names_its_replacement() {
    for (args, instead) in [
        (&["diff", "demo.md", "--exit-code"][..], "plan --check"),
        (&["serve", "demo.md", "--port", "0"][..], "prompt --preview"),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let said = String::from_utf8_lossy(&out.stderr);
        assert!(said.contains(instead), "{args:?}: {said}");
    }
}

/// Every JSON report says `ok`, true exactly when the command exits 0, so a
/// script can test one key; the timeline `plan` prints is a document, not a
/// report, and is printed as it is committed.
#[test]
fn every_json_report_says_ok() {
    let dir = tempdir();
    let json = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
            .current_dir(&dir)
            .args(["--format", "json"])
            .args(args)
            .output()
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{args:?}: {e}"));
        (out.status.code(), v)
    };
    let (code, new) = json(&["new", "p"]);
    assert_eq!((code, &new["ok"]), (Some(0), &serde_json::json!(true)));
    let script = "p/scripts/demo.md";
    let (_, plan) = json(&["plan", script]);
    assert!(plan.get("ok").is_none(), "the timeline is a document");
    let (code, drift) = json(&["plan", "--check", script]);
    assert_eq!((code, &drift["ok"]), (Some(3), &serde_json::json!(false)));
    std::fs::create_dir_all(dir.join("p/timelines")).unwrap();
    std::fs::write(
        dir.join("p/timelines/demo.en.json"),
        serde_json::to_string(&plan).unwrap(),
    )
    .unwrap();
    let (code, clean) = json(&["plan", "--check", script]);
    assert_eq!((code, &clean["ok"]), (Some(0), &serde_json::json!(true)));
    let (code, edit) = json(&["edit", script, "hold", "welcome-a"]);
    assert_eq!((code, &edit["ok"]), (Some(0), &serde_json::json!(true)));
}
