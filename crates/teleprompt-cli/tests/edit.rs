//! `teleprompt edit`: a timeline drag written into the script, and refused
//! whole when the script would no longer compile.

use std::path::Path;
use std::process::{Command, Output};

fn tp(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn action_ms(root: &Path, item: usize) -> u64 {
    let out = tp(root, &["plan", "scripts/demo.md", "--format", "json"]);
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    plan["entries"][item]["action"]["duration_ms"]
        .as_u64()
        .unwrap()
}

#[test]
fn a_stretch_lengthens_the_shot_in_the_plan() {
    let dir = teleprompt_testkit::test_dir("edit-stretch");
    teleprompt_cli::commands::new::scaffold(&dir).unwrap();
    assert_eq!(action_ms(&dir, 0), 500);
    let out = tp(
        &dir,
        &[
            "edit",
            "scripts/demo.md",
            "stretch",
            "welcome-a",
            "--by",
            "2",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(action_ms(&dir, 0), 1000);
    let md = std::fs::read_to_string(dir.join("scripts/demo.md")).unwrap();
    assert!(md.contains("```teleprompt scene=mock stretch=2\n"), "{md}");
}

#[test]
fn a_cue_starts_the_shot_on_its_word() {
    let dir = teleprompt_testkit::test_dir("edit-cue");
    teleprompt_cli::commands::new::scaffold(&dir).unwrap();
    let out = tp(
        &dir,
        &["edit", "scripts/demo.md", "cue", "welcome-a", "--word", "4"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let md = std::fs::read_to_string(dir.join("scripts/demo.md")).unwrap();
    assert!(
        md.contains("```teleprompt scene=mock policy=concurrent cue=\"paragraph is\"\n"),
        "{md}"
    );
}

/// Past `max_stretch` the script would not compile: nothing is written.
#[test]
fn an_edit_that_breaks_the_script_is_not_written() {
    let dir = teleprompt_testkit::test_dir("edit-refused");
    teleprompt_cli::commands::new::scaffold(&dir).unwrap();
    let before = std::fs::read_to_string(dir.join("scripts/demo.md")).unwrap();
    let out = tp(
        &dir,
        &[
            "edit",
            "scripts/demo.md",
            "stretch",
            "welcome-a",
            "--by",
            "10",
        ],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("not written") && err.contains("outside"),
        "{err}"
    );
    let after = std::fs::read_to_string(dir.join("scripts/demo.md")).unwrap();
    assert_eq!(before, after);
}

/// A line whose take was heard saying other words is reworded to them,
/// and its take is current for the new words: nothing to record again.
#[test]
fn said_rewords_a_line_to_what_its_take_says() {
    let dir = teleprompt_testkit::test_dir("edit-said");
    teleprompt_cli::commands::new::scaffold(&dir).unwrap();
    let text = "Welcome to teleprompt. This paragraph is a narration line, and its spoken \
                length decides how long the visuals below stay on screen.";
    let pcm = teleprompt_voice::Pcm {
        sample_rate: 16_000,
        channels: 1,
        samples: vec![0; 16_000],
    };
    let mut takes = teleprompt_voice::takes::Takes::load(&dir.join("takes")).unwrap();
    takes
        .save_heard(
            "welcome",
            text,
            "WELCOME TO TELEPROMPT THIS PARAGRAPH IS A NARRATION LINE",
            &pcm,
        )
        .unwrap();

    let out = tp(&dir, &["edit", "scripts/demo.md", "said", "welcome"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let said = "Welcome to teleprompt. This paragraph is a narration line.";
    let md = std::fs::read_to_string(dir.join("scripts/demo.md")).unwrap();
    assert!(md.contains(&format!("{said} {{#welcome}}")), "{md}");
    let takes = teleprompt_voice::takes::Takes::load(&dir.join("takes")).unwrap();
    assert!(takes.current("welcome", said).is_some());

    // Said as it now reads: nothing to reword.
    let again = tp(&dir, &["edit", "scripts/demo.md", "said", "welcome"]);
    assert!(!again.status.success());
    let err = String::from_utf8_lossy(&again.stderr);
    assert!(err.contains("welcome"), "{err}");
}

/// Refused, the edit says why as a list an app can show line by line, and
/// leaves nothing behind but the script as it was.
#[test]
fn a_refused_edit_lists_its_reasons_and_leaves_no_file() {
    let dir = teleprompt_testkit::test_dir("edit-refused-json");
    teleprompt_cli::commands::new::scaffold(&dir).unwrap();
    let args = [
        "--format",
        "json",
        "edit",
        "scripts/demo.md",
        "stretch",
        "welcome-a",
        "--by",
        "10",
    ];
    let out = tp(&dir, &args);
    assert_eq!(out.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["ok"], false);
    let errors = report["errors"].as_array().unwrap();
    assert!(errors.len() >= 2, "{errors:?}");
    assert!(errors[0].as_str().unwrap().starts_with("not written"));
    assert!(errors[1..]
        .iter()
        .any(|e| e.as_str().unwrap().contains("outside")));
    let left: Vec<_> = std::fs::read_dir(dir.join("scripts"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["demo.md"]);
}

/// An edit naming no block in the script is the script's problem: 2.
#[test]
fn an_edit_of_an_unknown_block_is_invalid() {
    let dir = teleprompt_testkit::test_dir("edit-unknown");
    teleprompt_cli::commands::new::scaffold(&dir).unwrap();
    let out = tp(&dir, &["edit", "scripts/demo.md", "hold", "no-such-block"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no-such-block"));
}

/// A script that is a link is edited where it points, and keeps its mode:
/// the edit replaces the file's words, not the file.
#[cfg(unix)]
#[test]
fn an_edit_writes_through_a_link_and_keeps_the_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = teleprompt_testkit::test_dir("edit-link");
    teleprompt_cli::commands::new::scaffold(&dir).unwrap();
    let scripts = dir.join("scripts");
    std::fs::rename(scripts.join("demo.md"), scripts.join("demo.real.md")).unwrap();
    std::os::unix::fs::symlink("demo.real.md", scripts.join("demo.md")).unwrap();
    let mode = std::fs::Permissions::from_mode(0o640);
    std::fs::set_permissions(scripts.join("demo.real.md"), mode).unwrap();
    let before = std::fs::read_to_string(scripts.join("demo.real.md")).unwrap();

    let args = [
        "edit",
        "scripts/demo.md",
        "stretch",
        "welcome-a",
        "--by",
        "2",
    ];
    let out = tp(&dir, &args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let link = std::fs::symlink_metadata(scripts.join("demo.md")).unwrap();
    assert!(link.file_type().is_symlink(), "the link is still a link");
    let real = scripts.join("demo.real.md");
    assert_ne!(std::fs::read_to_string(&real).unwrap(), before);
    let kept = std::fs::metadata(&real).unwrap().permissions().mode() & 0o777;
    assert_eq!(kept, 0o640);
}

#[test]
fn a_line_is_reworded_and_told_how_to_sound() {
    let dir = teleprompt_testkit::test_dir("edit-reword");
    teleprompt_cli::commands::new::scaffold(&dir).unwrap();
    let ok = |args: &[&str]| {
        let out = tp(&dir, args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    ok(&[
        "edit",
        "scripts/demo.md",
        "reword",
        "welcome",
        "Hello, and welcome.",
    ]);
    ok(&["edit", "scripts/demo.md", "instruct", "welcome", "brightly"]);
    let md = std::fs::read_to_string(dir.join("scripts/demo.md")).unwrap();
    assert!(
        md.contains("Hello, and welcome. {#welcome voice.instruct=brightly}"),
        "{md}"
    );
    ok(&["edit", "scripts/demo.md", "instruct", "welcome"]);
    let md = std::fs::read_to_string(dir.join("scripts/demo.md")).unwrap();
    assert!(md.contains("Hello, and welcome. {#welcome}"), "{md}");
}
