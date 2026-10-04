//! The recording scene: an asciinema cast, split at its own markers.

use teleprompt_asciinema::capture::session_cast;
use teleprompt_asciinema::scene::{parse, retimed, split};
use teleprompt_asciinema::AsciinemaScene;
use teleprompt_core::{BlockId, Hash};
use teleprompt_plugin::capture::{Session, SessionShot};
use teleprompt_plugin::scene::contract::{BlockSource, BodyOrigin, Measured, SceneCompiler, Shot};

const V2: &str = r#"{"version": 2, "width": 80, "height": 24}
[0.5, "o", "$ "]
[1.0, "o", "ls\r\n"]
[1.2, "i", "ignored input"]
[2.0, "m", "listed"]
[2.5, "o", "a  b  c\r\n"]
[4.0, "o", "$ "]
"#;

fn src(body: &str) -> BlockSource {
    BlockSource {
        scene: "rec".into(),
        body: body.into(),
        origin: BodyOrigin::Included {
            path: "casts/demo.cast".into(),
        },
    }
}

fn shots(body: &str) -> Vec<Shot> {
    let v = AsciinemaScene.validate(&src(body)).expect("valid");
    AsciinemaScene
        .shots(&v, &BlockId::from("b"))
        .expect("shots")
}

#[test]
fn a_v2_cast_is_read_with_input_dropped() {
    let c = parse(V2).unwrap();
    assert_eq!((c.width, c.height), (80, 24));
    assert_eq!(c.duration, 4.0);
    assert!(c.events.iter().all(|e| e.code != "i"));
}

/// v3 times are intervals since the previous event, and its size is under
/// `term`.
#[test]
fn a_v3_cast_is_read_by_its_intervals() {
    let v3 = "{\"version\": 3, \"term\": {\"cols\": 100, \"rows\": 30}}\n\
              [0.5, \"o\", \"a\"]\n[1.5, \"o\", \"b\"]\n# a comment\n[0.25, \"x\", \"0\"]\n";
    let c = parse(v3).unwrap();
    assert_eq!((c.width, c.height), (100, 30));
    assert_eq!(
        c.events.iter().map(|e| e.time).collect::<Vec<_>>(),
        [0.5, 2.0]
    );
}

/// Idle time is capped as `asciinema play` caps it, so the shot lasts as
/// long as it plays.
#[test]
fn the_idle_time_limit_is_applied() {
    let cast = "{\"version\": 2, \"width\": 80, \"height\": 24, \"idle_time_limit\": 1}\n\
                [0.5, \"o\", \"a\"]\n[30.5, \"o\", \"b\"]\n";
    assert_eq!(parse(cast).unwrap().duration, 1.5);
}

/// The cast's own markers split it; each shot is a cast of its own,
/// rebased to zero and stating its exact length.
#[test]
fn markers_split_a_cast_into_exact_shots() {
    let s = shots(V2);
    assert_eq!(s.len(), 2);
    assert_eq!(AsciinemaScene.estimate(&s[0]), Measured::Exact(2000));
    assert_eq!(AsciinemaScene.estimate(&s[1]), Measured::Exact(2000));
    let second = parse(&s[1].source).unwrap();
    assert_eq!(second.events[0].time, 0.5, "rebased: {}", s[1].source);
    assert!(
        !s[0].source.contains("\"m\""),
        "the marker is the split, not an event"
    );
}

#[test]
fn a_marker_at_the_start_marks_nothing() {
    let cast =
        "{\"version\": 2, \"width\": 80, \"height\": 24}\n[0, \"m\", \"\"]\n[1, \"o\", \"a\"]\n";
    assert_eq!(split(&parse(cast).unwrap()).len(), 1);
}

/// A shot after a resize opens at the size the terminal had by then.
#[test]
fn a_shot_opens_at_the_size_the_terminal_had() {
    let cast = "{\"version\": 2, \"width\": 80, \"height\": 24}\n\
                [0.5, \"r\", \"120x40\"]\n[1, \"m\", \"\"]\n[2, \"o\", \"a\"]\n";
    let parts = split(&parse(cast).unwrap());
    assert_eq!((parts[1].width, parts[1].height), (120, 40));
}

/// Stretching or trimming moves the pauses, never the typing.
#[test]
fn re_timing_moves_pauses_not_keystrokes() {
    let cast = "{\"version\": 2, \"width\": 80, \"height\": 24}\n\
                [0.05, \"o\", \"l\"]\n[0.1, \"o\", \"s\"]\n[2.1, \"o\", \"out\"]\n";
    let c = parse(cast).unwrap();
    let long = retimed(&c, 4.1).unwrap();
    assert_eq!(long.duration, 4.1);
    assert!(
        (long.events[1].time - long.events[0].time - 0.05).abs() < 1e-9,
        "typing untouched"
    );
    assert!(
        (long.events[2].time - 4.1).abs() < 1e-9,
        "the pause took it all: {long:?}"
    );

    let short = retimed(&c, 1.0).unwrap();
    assert!((short.events[2].time - 1.0).abs() < 1e-9, "{short:?}");
    assert!(
        retimed(&c, 0.2).is_none(),
        "the pause cannot go below typing speed"
    );
}

#[test]
fn a_shot_is_re_timed_to_its_slot() {
    let s = &shots(V2)[1];
    let source = AsciinemaScene.retime(s, 3_000).expect("re-timed");
    assert_eq!(parse(&source).unwrap().duration, 3.0);
}

/// Bad lines are reported in the cast file, at their own line numbers.
#[test]
fn a_bad_line_is_reported_in_the_cast_file() {
    let diags = AsciinemaScene
        .validate(&src(
            "{\"version\": 2, \"width\": 80, \"height\": 24}\n[0.5, \"o\"\nnope\n",
        ))
        .expect_err("refused");
    assert_eq!(diags.len(), 2, "{diags:#?}");
    assert_eq!(diags[0].span.unwrap().line, 2);
    assert_eq!(diags[0].file.as_deref(), Some("casts/demo.cast"));
    assert!(AsciinemaScene.validate(&src("{\"version\": 1}\n")).is_err());
}

/// A terminal shot opens on the screen before it, so shots chain.
#[test]
fn a_shot_continues_the_one_before_it() {
    assert!(AsciinemaScene.continues());
}

/// The session is replayed from its start, each shot at its scheduled
/// offset, held until its slot ends.
#[test]
fn the_session_lays_shots_at_their_scheduled_offsets() {
    let parts = shots(V2);
    let session = Session {
        scene: "rec".into(),
        adapter: "asciinema".into(),
        name: None,
        settings: Default::default(),
        shots: parts
            .iter()
            .zip([3_000, 2_000])
            .map(|(s, ms)| SessionShot {
                id: s.id.clone(),
                key: Hash::of(s.id.as_bytes()),
                source: s.source.clone(),
                duration_ms: ms,
                wanted: true,
            })
            .collect(),
    };
    let cast = session_cast(&session).unwrap();
    assert_eq!(cast.duration, 5.0);
    // The second shot's first output lands 0.5s into its slot at 3s.
    assert!(cast
        .events
        .iter()
        .any(|e| e.data == "a  b  c\r\n" && (e.time - 3.5).abs() < 1e-9));
}

mod select {
    use teleprompt_asciinema::scene::{parse, select, split};

    const CAST: &str = "{\"version\": 2, \"width\": 80, \"height\": 24}\n\
        [0.5, \"o\", \"one\"]\n[1, \"m\", \"\"]\n[1.5, \"o\", \"two\"]\n\
        [2, \"m\", \"plan\"]\n[2.5, \"o\", \"three\"]\n[3, \"o\", \"end\"]\n";

    fn outputs(fragment: &str) -> Vec<String> {
        let c = select(&parse(CAST).unwrap(), fragment).unwrap();
        c.events
            .iter()
            .map(|e| format!("{}@{}", e.data, e.time))
            .collect()
    }

    /// `#2` is the part after the first marker, rebased to zero.
    #[test]
    fn a_number_selects_a_part() {
        assert_eq!(outputs("2"), ["two@0.5"]);
    }

    /// A marker's label selects the part it begins.
    #[test]
    fn a_label_selects_the_part_it_begins() {
        assert_eq!(outputs("plan"), ["three@0.5", "end@1"]);
    }

    /// A range keeps its parts' own splits, so each is still a shot.
    #[test]
    fn a_range_keeps_its_parts_apart() {
        let c = select(&parse(CAST).unwrap(), "1-2").unwrap();
        assert_eq!(split(&c).len(), 2);
        assert_eq!(c.duration, 2.0);
    }

    #[test]
    fn what_names_nothing_says_what_there_is() {
        let e = select(&parse(CAST).unwrap(), "deploy").unwrap_err();
        assert!(e.contains("3 part(s)") && e.contains("plan"), "{e}");
        assert!(select(&parse(CAST).unwrap(), "4").is_err());
    }

    /// A range back to front names nothing; it is not a crash.
    #[test]
    fn a_reversed_range_names_nothing() {
        assert!(select(&parse(CAST).unwrap(), "3-1").is_err());
    }
}
