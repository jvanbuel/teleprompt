//! The timeline as `plan` lays it out, and a stretch.

mod plan;

use plan::{line, shot};
use teleprompt_core::DurationSource::{Exact, Unknown};
use teleprompt_gtk::timeline::{parse, stretched, Edit, Timeline};

/// Two lines: "welcome" (150–9800 ms, recorded) with a shot held after
/// it, and "the-loop" (10000–18600 ms) with a shot at its start, and an
/// untimed browser shot held after it.
fn timeline() -> Timeline {
    let welcome = line("welcome", 150, 9650);
    let welcome = plan::NarrationEntry {
        recorded: true,
        ..welcome
    };
    parse(&plan::json(
        24000,
        vec![
            (
                Some(welcome),
                Some(shot("welcome-a#0", "vhs", 9950, 500, Exact)),
            ),
            (
                Some(line("the-loop", 10000, 8600)),
                Some(shot("the-loop-a#0", "vhs", 10000, 800, Exact)),
            ),
            (
                None,
                Some(shot("the-loop-a2#0", "playwright", 18800, 5000, Unknown)),
            ),
        ],
    ))
    .unwrap()
}

#[test]
fn lines_and_shots_are_read_in_time() {
    let t = timeline();
    assert_eq!(t.duration_ms, 24000);
    assert_eq!(t.lines.len(), 2);
    assert!(t.lines[0].recorded && !t.lines[1].recorded);
    assert_eq!((t.lines[1].start_ms, t.lines[1].end_ms), (10000, 18600));
    assert_eq!(t.shots[0].block(), "welcome-a");
    assert_eq!(t.shots[0].line.as_deref(), Some("welcome"));
    assert_eq!(t.shots[2].line, None);
    assert!(t.shots[0].timed && !t.shots[2].timed);
}

#[test]
fn only_a_shot_that_states_its_length_stretches() {
    let t = timeline();
    // 500 ms to 750 ms.
    let d = stretched(&t.shots[0], 9950 + 750).unwrap();
    assert_eq!(d.args(), ["stretch", "welcome-a", "--by", "1.500"]);
    assert_eq!(stretched(&t.shots[2], 30000), None);
    // A twitch is not a stretch.
    assert_eq!(stretched(&t.shots[0], 9950 + 510), None);
}

#[test]
fn a_plan_missing_a_field_is_an_error_not_zero() {
    let partial = r#"{"duration_ms": 24000, "entries": [
      {"action": {"shot": "a-a#0", "scene": "vhs", "start_ms": 10}}]}"#;
    assert!(parse(partial).is_err());
}

#[test]
fn a_blocks_first_shot_leads() {
    let t = timeline();
    assert!(t.shots[0].leads());
    let second = teleprompt_gtk::timeline::ShotSpan {
        shot: "welcome-a#1".into(),
        ..t.shots[0].clone()
    };
    assert!(!second.leads());
    assert_eq!(second.block(), "welcome-a");
}

#[test]
fn a_line_is_reworded_or_directed_by_its_id() {
    let reword = Edit::Reword {
        line: "welcome".into(),
        text: "-- Hello.".into(),
    };
    assert_eq!(reword.args(), ["reword", "--", "welcome", "-- Hello."]);
    let instruct = Edit::Instruct {
        line: "welcome".into(),
        text: Some("warmly".into()),
    };
    assert_eq!(instruct.args(), ["instruct", "--", "welcome", "warmly"]);
    let plain = Edit::Instruct {
        line: "welcome".into(),
        text: None,
    };
    assert_eq!(plain.args(), ["instruct", "--", "welcome"]);
}
