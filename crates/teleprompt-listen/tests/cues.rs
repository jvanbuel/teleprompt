//! Cues: points in a script where something happens once the reader gets
//! there, such as a shot starting.

use teleprompt_listen::{Cues, Position};

fn at(line: usize, word: usize) -> Position {
    Position { line, word }
}

#[test]
fn a_cue_fires_once_when_the_reader_reaches_it() {
    let mut cues = Cues::new(vec![at(0, 3)]);
    assert_eq!(cues.reach(at(0, 2)), 0..0);
    assert_eq!(cues.reach(at(0, 3)), 0..1);
    assert_eq!(cues.reach(at(0, 4)), 1..1);
}

/// A reader who is heard late, or reads fast, passes several at once; they
/// fire together, in script order.
#[test]
fn passing_several_cues_fires_them_in_order() {
    let mut cues = Cues::new(vec![at(0, 1), at(1, 0), at(1, 0), at(2, 0)]);
    assert_eq!(cues.reach(at(1, 2)), 0..3);
    assert_eq!(cues.reach(at(3, 0)), 3..4);
}

/// A recognizer revises its hypothesis, so the reader can seem to step back;
/// what fired stays fired.
#[test]
fn stepping_back_does_not_fire_a_cue_again() {
    let mut cues = Cues::new(vec![at(0, 2), at(0, 4)]);
    assert_eq!(cues.reach(at(0, 3)), 0..1);
    assert_eq!(cues.reach(at(0, 1)), 1..1);
    assert_eq!(cues.reach(at(0, 3)), 1..1);
}

/// A new take starts from the top, and a cue at the very top fires as it
/// starts, before a word is said.
#[test]
fn a_restart_arms_every_cue_again() {
    let mut cues = Cues::new(vec![at(0, 0), at(0, 2)]);
    assert_eq!(cues.reach(at(1, 0)), 0..2);
    cues.restart();
    assert_eq!(cues.reach(at(0, 0)), 0..1);
}

/// A take started part-way through fires only what lies ahead of it; what
/// comes before it in the script is not played all at once.
#[test]
fn a_restart_at_a_line_arms_only_the_cues_from_there() {
    let mut cues = Cues::new(vec![at(0, 0), at(0, 3), at(1, 0), at(1, 2)]);
    cues.restart_at(at(1, 0));
    assert_eq!(cues.reach(at(1, 0)), 2..3);
    assert_eq!(cues.reach(at(1, 2)), 3..4);
}
