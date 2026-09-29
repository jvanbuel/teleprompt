//! The retake queue: the lines reworded since their takes, re-recorded
//! one after another, each kept once the reader has moved past it.

use teleprompt_gtk::api::{Line, Position, Script};
use teleprompt_gtk::retake::Queue;

fn script(stale: &[bool]) -> Script {
    Script {
        lines: stale
            .iter()
            .enumerate()
            .map(|(i, &stale)| Line {
                id: format!("l{i}"),
                text: "Some words here.".into(),
                recorded: !stale,
                stale,
                said: None,
                said_diff: Vec::new(),
                audio: None,
                instruct: None,
            })
            .collect(),
        shots: Vec::new(),
        ..Script::default()
    }
}

fn at(line: usize, word: usize) -> Position {
    Position { line, word }
}

#[test]
fn the_queue_is_the_reworded_lines_in_order() {
    let q = Queue::of(&script(&[false, true, false, true]));
    assert_eq!(q.len(), 2);
    assert_eq!(q.line(), Some(1));
    assert!(Queue::of(&script(&[false, false])).is_empty());
}

#[test]
fn a_line_is_read_once_the_reader_moves_past_it() {
    let q = Queue::of(&script(&[true, true]));
    assert!(!q.read(at(0, 2)));
    assert!(q.read(at(1, 0)));
}

#[test]
fn after_each_line_the_next_reworded_one() {
    let mut q = Queue::of(&script(&[true, false, true]));
    assert_eq!(q.says(), "1 of 2");
    assert_eq!(q.advance(), Some(2));
    assert_eq!(q.says(), "2 of 2");
    assert_eq!(q.advance(), None);
    assert_eq!(q.line(), None);
}

/// The last line has no next line to move on to: it is read when the
/// reader reaches its last word.
#[test]
fn the_script_s_last_line_is_read_at_its_last_word() {
    let q = Queue::of(&script(&[false, true]));
    assert!(!q.read(at(1, 1)));
    assert!(q.read(at(1, 3)));
}
