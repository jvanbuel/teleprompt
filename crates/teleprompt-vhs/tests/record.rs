//! A tape `vhs record` wrote, read back as the commands in it.

use teleprompt_plugin::scene::SceneCompiler;
use teleprompt_vhs::record::read;
use teleprompt_vhs::VhsScene;

/// As `vhs record` writes it.
const TAPE: &str = "Sleep 1s\nType \"ls\"\nEnter\nSleep 1.5s\nType \"echo hi\"\nEnter\nSleep 2s\nType \"exit\"\nEnter\n";

#[test]
fn each_command_is_a_step_timed_as_vhs_plays_it() {
    let r = read(TAPE);
    assert_eq!(r.head, "Sleep 1s\n");
    let starts: Vec<u64> = r.steps.iter().map(|s| s.start_ms).collect();
    // 1s, then "ls" and Enter at 50ms a key, then 1.5s.
    assert_eq!(starts, [1000, 1000 + 150 + 1500]);
    assert_eq!(r.steps[0].text, "Type \"ls\"\nEnter\nSleep 1.5s\n");
}

#[test]
fn the_closing_exit_is_left_out() {
    assert!(!read(TAPE).marked(&[]).contains("exit"));
}

#[test]
fn a_cut_is_a_mark_the_scene_selects_by() {
    let marked = read(TAPE).marked(&[1]);
    assert_eq!(
        VhsScene.select(&marked, "2").unwrap(),
        "Type \"echo hi\"\nEnter\nSleep 2s\n"
    );
}
