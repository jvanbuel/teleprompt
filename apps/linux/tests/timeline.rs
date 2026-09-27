//! The timeline as `plan` lays it out, and a stretch.

use teleprompt_gtk::timeline::{parse, stretched};

/// Two lines: "welcome" (150–9800 ms, recorded) with a shot held after
/// it, and "the-loop" (10000–18600 ms) with a shot at its start, and an
/// untimed browser shot held after it.
const PLAN: &str = r#"{"duration_ms": 24000, "entries": [
 {"narration": {"line": "welcome", "start_ms": 150, "duration_ms": 9650, "recorded": true},
  "action": {"shot": "welcome-a#0", "scene": "vhs", "start_ms": 9950, "duration_ms": 500, "duration_source": "exact"}},
 {"narration": {"line": "the-loop", "start_ms": 10000, "duration_ms": 8600},
  "action": {"shot": "the-loop-a#0", "scene": "vhs", "start_ms": 10000, "duration_ms": 800, "duration_source": "exact"}},
 {"action": {"shot": "the-loop-a2#0", "scene": "playwright", "start_ms": 18800, "duration_ms": 5000, "duration_source": "unknown"}}
]}"#;

#[test]
fn lines_and_shots_are_read_in_time() {
    let t = parse(PLAN).unwrap();
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
    let t = parse(PLAN).unwrap();
    // 500 ms to 750 ms.
    let d = stretched(&t.shots[0], 9950 + 750).unwrap();
    assert_eq!(d.args(), ["stretch", "welcome-a", "--by", "1.500"]);
    assert_eq!(stretched(&t.shots[2], 30000), None);
    // A twitch is not a stretch.
    assert_eq!(stretched(&t.shots[0], 9950 + 510), None);
}
