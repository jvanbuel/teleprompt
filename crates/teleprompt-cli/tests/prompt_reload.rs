//! A script edited while it is read: the prompter reloads it, shots moved
//! and lines reworded alike.

use teleprompt_cli::cmd::prompt::{edits_of, reload_on_edit};
use teleprompt_cli::project::Project;

/// Past a file system's timestamp granularity, so an edit is seen.
fn later() {
    std::thread::sleep(std::time::Duration::from_millis(1100));
}

#[test]
fn an_edited_script_is_reloaded_shots_and_lines_alike() {
    let dir = teleprompt_testkit::test_dir("prompt-reload-edit");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/demo.md");
    let project = Project::for_script(&script).unwrap();
    let reload = reload_on_edit(&project, &script, "en");

    // Unchanged: nothing to place.
    assert!(reload().is_none());

    later();
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(
        &script,
        text.replacen(
            "```teleprompt scene=mock\n",
            "```teleprompt scene=mock policy=concurrent cue=\"This paragraph\"\n",
            1,
        ),
    )
    .unwrap();
    let prompt = reload().expect("the script, reloaded");
    assert!(prompt.shots[0].at.word > 0, "{:?}", prompt.shots[0].at);
    // Asked again without an edit since: nothing new.
    assert!(reload().is_none());

    later();
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(
        &script,
        text.replacen("Welcome to teleprompt.", "Hello there.", 1),
    )
    .unwrap();
    let prompt = reload().expect("reworded lines, reloaded");
    assert!(
        prompt.lines[0].starts_with("Hello there."),
        "{:?}",
        prompt.lines
    );
}

/// Keeping what a take was heard to say rewords the script's line, and
/// the prompter reloads it with the take current.
#[test]
fn keeping_what_was_said_rewords_the_script_and_reloads() {
    let dir = teleprompt_testkit::test_dir("prompt-keep-said");
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
    let script = dir.join("scripts/demo.md");
    let project = Project::for_script(&script).unwrap();
    let edits = edits_of(&project, &script, "en");

    later();
    (edits.keep_said)("welcome").unwrap();
    let said = "Welcome to teleprompt. This paragraph is a narration line.";
    let prompt = (edits.reload)().expect("the reworded script");
    assert_eq!(prompt.lines[0], said);
    let takes = teleprompt_voice::takes::Takes::load(&dir.join("takes")).unwrap();
    assert!(takes.current("welcome", said).is_some());

    // Said as it now reads: nothing to keep.
    assert!((edits.keep_said)("welcome")
        .unwrap_err()
        .contains("welcome"));
}
