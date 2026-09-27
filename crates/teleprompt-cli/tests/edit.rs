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
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
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
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
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
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
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
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
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
