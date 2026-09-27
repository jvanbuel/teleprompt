//! A script edited while it is read: the prompter places its shots again
//! when only its shots changed, and leaves them when its lines did.

use teleprompt_cli::cmd::check::compile_script;
use teleprompt_cli::cmd::prompt::reload_on_edit;
use teleprompt_cli::project::Project;

/// Past a file system's timestamp granularity, so an edit is seen.
fn later() {
    std::thread::sleep(std::time::Duration::from_millis(1100));
}

#[test]
fn a_moved_shot_is_placed_again_and_changed_lines_are_not() {
    let dir = teleprompt_testkit::test_dir("prompt-reload-edit");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/demo.md");
    let project = Project::for_script(&script).unwrap();
    let (compiled, _) = compile_script(&project, &script, "en").unwrap();
    let lines: Vec<String> = compiled.narration.iter().map(|n| n.text.clone()).collect();
    let reload = reload_on_edit(&project, &script, "en", &lines);

    // Unchanged: nothing to place.
    assert!(reload().is_none());

    later();
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, text.replacen("```teleprompt scene=mock\n", "```teleprompt scene=mock policy=concurrent cue=\"This paragraph\"\n", 1)).unwrap();
    let shots = reload().expect("the shots, placed again");
    assert!(shots[0].at.word > 0, "{:?}", shots[0].at);
    // Asked again without an edit since: nothing new.
    assert!(reload().is_none());

    later();
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(&script, text.replacen("Welcome to teleprompt.", "Hello there.", 1)).unwrap();
    assert!(reload().is_none(), "changed lines need the script reopened");
}
