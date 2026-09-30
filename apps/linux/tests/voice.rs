//! A script read by its voice: where the reading is at a point in a line's
//! audio, which shots are due, and how far the voice has got making the
//! lines.

mod common;

use common::example;
use teleprompt_gtk::api::{Position, Script};
use teleprompt_gtk::voice::{self, Mark};

fn voiced() -> Script {
    serde_json::from_str(&example("voiced_script.json")).unwrap()
}

#[test]
fn the_word_being_said_is_the_last_one_begun() {
    let starts = [0, 520, 660, 1100];
    assert_eq!(voice::word_at(&starts, 0), 0);
    assert_eq!(voice::word_at(&starts, 519), 0);
    assert_eq!(voice::word_at(&starts, 520), 1);
    assert_eq!(voice::word_at(&starts, 5_000), 3);
    assert_eq!(voice::word_at(&[], 300), 0);
}

#[test]
fn shots_are_due_as_the_reading_reaches_them_and_once() {
    let script = voiced();
    let pos = |line, word| Position { line, word };
    let mut started = std::collections::BTreeSet::new();
    // Starting on the first line plays the shot cued there.
    assert_eq!(
        voice::due(&script, pos(0, 0), pos(0, 0), &started),
        ["intro#0"]
    );
    started.insert("intro#0".to_string());
    assert!(voice::due(&script, pos(0, 0), pos(0, 7), &started).is_empty());
    assert_eq!(
        voice::due(&script, pos(0, 0), pos(1, 0), &started),
        ["welcome-b#0"]
    );
    // Starting past a shot's cue does not play it.
    let none = std::collections::BTreeSet::new();
    assert_eq!(
        voice::due(&script, pos(1, 0), pos(1, 2), &none),
        ["welcome-b#0"]
    );
}

#[test]
fn each_line_is_marked_by_where_its_audio_comes_from() {
    let mut script = voiced();
    assert_eq!(voice::mark(&script.lines[0]), Some(Mark::Voiced));
    assert_eq!(voice::mark(&script.lines[1]), Some(Mark::Unvoiced));
    let audio = script.lines[1].audio.as_mut().unwrap();
    audio.source = teleprompt_gtk::api::Source::Take;
    assert_eq!(voice::mark(&script.lines[1]), Some(Mark::Take));
    let plain: Script = serde_json::from_str(&example("script.json")).unwrap();
    assert_eq!(voice::mark(&plain.lines[0]), None);
}

#[test]
fn progress_counts_the_lines_the_voice_has_made() {
    let script = voiced();
    assert_eq!(voice::made(&script), (1, 2));
    assert_eq!(voice::unmade(&script), [1]);
}

#[test]
fn a_length_reads_as_minutes_and_seconds() {
    assert_eq!(voice::clock(6_120), "0:06");
    assert_eq!(voice::clock(83_400), "1:23");
    assert_eq!(voice::clock(3_725_000), "62:05");
}

#[test]
fn a_speakers_mark_is_their_initial_in_their_own_colour() {
    let script = voiced();
    assert_eq!(script.lines[0].speaker, None);
    assert_eq!(script.lines[1].speaker.as_deref(), Some("guest"));
    assert_eq!(voice::initial("guest"), "G");
    assert_eq!(voice::initial("émile"), "É");
    // The same name, the same colour, in every app and every run.
    assert_eq!(
        voice::speaker_colour("guest"),
        voice::speaker_colour("guest")
    );
    assert_ne!(voice::speaker_colour("guest"), voice::speaker_colour("me"));
    for name in ["guest", "me", "host", "ann"] {
        let hex = voice::speaker_colour(name);
        assert!(voice::SPEAKER_COLOURS.contains(&hex), "{name}: {hex}");
    }
}
