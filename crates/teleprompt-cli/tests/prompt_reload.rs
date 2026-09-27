//! A script edited while it is read: the prompter reloads it, shots moved
//! and lines reworded alike.

use teleprompt_cli::cmd::prompt::reload_on_edit;
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
