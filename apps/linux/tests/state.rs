mod common;

use std::time::Instant;

use teleprompt_gtk::api::{Position, Script, ServerMessage};
use teleprompt_gtk::state::{PrompterState, Status, Word};

fn loaded() -> PrompterState {
    let mut state = PrompterState::default();
    state.load(serde_json::from_str::<Script>(&common::example("script.json")).unwrap());
    state
}

fn reached(line: usize, word: usize, play: &[&str]) -> ServerMessage {
    ServerMessage::Reached {
        at: Position { line, word },
        play: play.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn words_before_the_reader_are_said() {
    let mut state = loaded();
    state.apply(reached(0, 3, &[]));
    assert_eq!(state.word(0, 2), Word::Said);
    assert_eq!(state.word(0, 3), Word::Next);
    assert_eq!(state.word(1, 0), Word::Ahead);
}

/// Shots play in the order given; reading on cuts off the one playing.
#[test]
fn shots_play_in_order_and_reading_on_cuts_them_off() {
    let mut state = loaded();
    state.start_take(0, Instant::now());
    state.apply(reached(0, 3, &["intro#0", "welcome-a#0"]));
    assert_eq!(state.playing.as_deref(), Some("intro#0"));
    assert_eq!(state.playing_clip(), None, "intro was never captured");
    state.clip_ended();
    assert_eq!(state.playing.as_deref(), Some("welcome-a#0"));
    assert!(state.playing_clip().unwrap().starts_with("/api/v1/clips/"));
    state.apply(reached(1, 0, &["welcome-b#0"]));
    assert_eq!(state.playing.as_deref(), Some("welcome-b#0"));
    state.clip_ended();
    assert_eq!(state.playing, None);
    assert_eq!(state.started.len(), 3);
}

#[test]
fn a_new_take_forgets_what_played() {
    let mut state = loaded();
    state.start_take(0, Instant::now());
    state.apply(reached(0, 3, &["intro#0"]));
    state.start_take(1, Instant::now());
    assert_eq!(state.at, Position { line: 1, word: 0 });
    assert_eq!(state.playing, None);
    assert!(state.started.is_empty());
    assert!(state.take.is_sending());
}

#[test]
fn stopping_says_what_was_kept() {
    let mut state = loaded();
    state.start_take(0, Instant::now());
    state.apply(ServerMessage::Stopped {
        saved: vec!["welcome".into()],
    });
    assert!(!state.take.is_taking());
    assert_eq!(state.status, Status::info("Kept line 1"));
    assert!(state.undoable);
    state.apply(ServerMessage::Stopped {
        saved: vec!["welcome".into(), "deploy".into()],
    });
    assert_eq!(state.status, Status::info("Kept lines 1, 2"));
    state.apply(ServerMessage::Stopped { saved: vec![] });
    assert_eq!(
        state.status,
        Status::info("Nothing kept: a line is kept once it is read to its end")
    );
    assert!(!state.undoable, "nothing to put back");
}

/// A take thrown away keeps nothing and says so; one kept and undone says
/// which lines were put back, and cannot be undone twice.
#[test]
fn a_take_discarded_or_undone_says_so() {
    let mut state = loaded();
    state.start_take(0, Instant::now());
    state.apply(ServerMessage::Discarded);
    assert!(!state.take.is_taking());
    assert_eq!(state.status, Status::info("Take discarded: nothing kept"));

    state.start_take(0, Instant::now());
    state.apply(ServerMessage::Stopped {
        saved: vec!["deploy".into()],
    });
    state.apply(ServerMessage::Undone {
        lines: vec!["deploy".into()],
    });
    assert_eq!(state.status, Status::info("Put back line 2 as it was"));
    assert!(!state.undoable);
    state.apply(ServerMessage::Undone {
        lines: vec!["welcome".into(), "deploy".into()],
    });
    assert_eq!(
        state.status,
        Status::info("Put back lines 1, 2 as they were")
    );
}

#[test]
fn an_error_is_shown() {
    let mut state = loaded();
    state.apply(ServerMessage::Error("disk full".into()));
    assert_eq!(state.status, Status::error("disk full"));
}

#[test]
fn the_end_of_the_script_is_said() {
    let mut state = loaded();
    state.apply(reached(2, 0, &[]));
    assert_eq!(state.status, Status::info("End of script"));
}

/// A take kept and the next counted in at once: the first take's `Stopped`
/// can arrive during the countdown, and must not call the countdown off.
#[test]
fn a_late_stopped_leaves_the_next_countdown_alone() {
    let mut state = loaded();
    state.start_take(0, std::time::Instant::now());
    state.take.stop(std::time::Instant::now());
    assert!(state.take.count());
    state.apply(ServerMessage::Stopped {
        saved: vec!["welcome".into()],
    });
    assert!(state.take.is_counting());
}
