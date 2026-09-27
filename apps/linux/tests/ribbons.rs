//! The glass as the timeline: where each shot lies under the words it
//! plays over, the moment each word is said, and what a drop on a word or
//! in the pause after a line asks of the script.

use teleprompt_gtk::ribbons::{
    dropped_after, dropped_on, moment, on_screen, ribbons, stretched_to, Ribbon,
};
use teleprompt_gtk::timeline::{parse, Edit, Timeline};

/// "One two six ten" 0–4000 ms, four words of equal letters, a shot
/// from its third word running 1500 ms past it; "Five six." 5000–7000 ms
/// with a shot held 2000 ms after it; a browser shot with no length of
/// its own held after that.
const PLAN: &str = r#"{"duration_ms": 12000, "entries": [
 {"narration": {"line": "a", "start_ms": 0, "duration_ms": 4000},
  "action": {"shot": "a-a#0", "scene": "vhs", "start_ms": 2000, "duration_ms": 3500, "duration_source": "exact"}},
 {"narration": {"line": "b", "start_ms": 5000, "duration_ms": 2000},
  "action": {"shot": "b-a#0", "scene": "vhs", "start_ms": 7000, "duration_ms": 2000, "duration_source": "exact"}}
]}"#;

const LINES: [(&str, &str); 2] = [("a", "One two six ten"), ("b", "Five six.")];

fn timeline() -> Timeline {
    parse(PLAN).unwrap()
}

fn lines() -> Vec<(String, String)> {
    LINES
        .iter()
        .map(|(i, t)| (i.to_string(), t.to_string()))
        .collect()
}

#[test]
fn a_shot_lies_under_the_words_it_plays_over() {
    let r = ribbons(&timeline(), &lines());
    assert_eq!(
        r[0],
        Ribbon {
            shot: 0,
            line: 0,
            words: 2..4,
            past_ms: 1500,
        }
    );
}

#[test]
fn a_held_shot_lies_in_the_pause_after_its_line() {
    let r = ribbons(&timeline(), &lines());
    assert_eq!(
        r[1],
        Ribbon {
            shot: 1,
            line: 1,
            words: 2..2,
            past_ms: 2000,
        }
    );
    assert!(r[1].held());
    assert!(!r[0].held());
}

#[test]
fn each_word_is_said_at_its_share_of_the_line() {
    let (t, l) = (timeline(), lines());
    assert_eq!(moment(&t, &l, 0, 0), 0);
    assert_eq!(moment(&t, &l, 0, 2), 2000);
    // "Five" and "six." share 2000 ms by their letters, five apiece.
    assert_eq!(moment(&t, &l, 1, 1), 6000);
}

#[test]
fn a_shot_dropped_on_a_word_starts_there() {
    let (t, l) = (timeline(), lines());
    let s = &t.shots[0];
    assert_eq!(
        dropped_on(s, &l, 0, 1),
        Edit::Cue {
            block: "a-a".into(),
            word: 1
        }
    );
    assert_eq!(
        dropped_on(s, &l, 1, 0),
        Edit::Move {
            block: "a-a".into(),
            after: "b".into(),
            word: Some(0)
        }
    );
}

#[test]
fn a_shot_dropped_in_a_pause_holds_after_that_line() {
    let (t, l) = (timeline(), lines());
    assert_eq!(
        dropped_after(&t.shots[0], &l, 0),
        Edit::Hold {
            block: "a-a".into()
        }
    );
    assert_eq!(
        dropped_after(&t.shots[1], &l, 0),
        Edit::Move {
            block: "b-a".into(),
            after: "a".into(),
            word: None
        }
    );
}

/// Its end dragged onto a word: as long as up to that word's end.
#[test]
fn a_shot_s_end_dragged_to_a_word_stretches_it_to_there() {
    let (t, l) = (timeline(), lines());
    // From 2000 to the end of "six" (3000): 1000 of its 3500 ms.
    let Some(Edit::Stretch { block, by }) = stretched_to(&t, &l, &t.shots[0], 0, 2) else {
        panic!("a stretch");
    };
    assert_eq!(block, "a-a");
    assert!((by - 1000.0 / 3500.0).abs() < 1e-9, "{by}");
    // On another line: no stretch.
    assert_eq!(stretched_to(&t, &l, &t.shots[0], 1, 0), None);
}

#[test]
fn a_moment_shows_the_shot_on_screen_then_and_how_far_into_it() {
    let t = timeline();
    // The third word of "a" is said at 2000 ms, as its shot starts.
    assert_eq!(on_screen(&t, 2000), Some((0, 0)));
    assert_eq!(on_screen(&t, moment(&t, &lines(), 0, 3)), Some((0, 1000)));
    // Before it, and between shots, nothing is.
    assert_eq!(on_screen(&t, 1000), None);
    assert_eq!(on_screen(&t, 6000), None);
    assert_eq!(on_screen(&t, 8500), Some((1, 1500)));
}
