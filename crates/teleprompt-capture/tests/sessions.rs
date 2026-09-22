//! Grouping a timeline's beats into the sessions a backend can run.
//!
//! All of it is arithmetic over a list: nothing here runs a terminal, and
//! the point of that is that the decisions a capture makes — what to run,
//! what to keep, where to stop — are testable without one.

use std::collections::HashSet;

use teleprompt_capture::{sessions, Beat};
use teleprompt_core::Hash;

fn beat(span: &str, scene: &str, source: &str) -> Beat {
    Beat {
        span: span.into(),
        scene: scene.into(),
        adapter: "mock".into(),
        session: None,
        key: Hash::of(span.as_bytes()),
        source: source.into(),
        duration_ms: 1_000,
    }
}

/// Nothing is cached.
fn cold(_: &Hash) -> bool {
    false
}

fn cached(spans: &[&str]) -> impl Fn(&Hash) -> bool {
    let keys: HashSet<Hash> = spans.iter().map(|s| Hash::of(s.as_bytes())).collect();
    move |key: &Hash| keys.contains(key)
}

#[test]
fn the_beats_of_a_scene_are_one_session_in_order() {
    let out = sessions(
        &[
            beat("a#0", "terminal", "wait 1s"),
            beat("b#0", "terminal", "wait 2s"),
        ],
        &cold,
    );

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].scene, "terminal");
    assert_eq!(out[0].steps.len(), 2);
    assert_eq!(out[0].steps[0].span, "a#0");
    assert!(out[0].steps.iter().all(|s| s.wanted));
}

#[test]
fn two_scenes_are_two_sessions() {
    let out = sessions(
        &[
            beat("a#0", "terminal", "wait 1s"),
            beat("b#0", "browser", "wait 1s"),
            beat("c#0", "terminal", "wait 1s"),
        ],
        &cold,
    );

    assert_eq!(out.len(), 2);
    assert_eq!(out[0].steps.len(), 2, "the terminal's two beats");
    assert_eq!(out[1].steps.len(), 1);
}

#[test]
fn a_named_session_is_a_session_of_its_own() {
    let mut second = beat("b#0", "terminal", "wait 1s");
    second.session = Some("retry".into());
    let out = sessions(&[beat("a#0", "terminal", "wait 1s"), second], &cold);

    assert_eq!(out.len(), 2);
    assert_eq!(out[1].name.as_deref(), Some("retry"));
}

/// The property that makes a session a session. A step whose clip is
/// already in hand still runs, because the step after it opens on the
/// screen it leaves behind — it just is not kept.
#[test]
fn a_cached_step_still_runs_when_something_after_it_is_wanted() {
    let out = sessions(
        &[
            beat("a#0", "terminal", "wait 1s"),
            beat("b#0", "terminal", "wait 1s"),
        ],
        &cached(&["a#0"]),
    );

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].steps.len(), 2, "both steps run");
    assert!(!out[0].steps[0].wanted, "the first one is not kept");
    assert!(out[0].steps[1].wanted);
    assert_eq!(out[0].wanted(), 1);
}

/// And the other end: a step after the last wanted one leads nowhere
/// anybody is looking. On a tape whose sleeps are real seconds that is the
/// difference between a capture that stops and one that sits there.
#[test]
fn a_session_stops_after_the_last_step_worth_keeping() {
    let out = sessions(
        &[
            beat("a#0", "terminal", "wait 1s"),
            beat("b#0", "terminal", "wait 1s"),
            beat("c#0", "terminal", "wait 1s"),
        ],
        &cached(&["b#0", "c#0"]),
    );

    assert_eq!(out[0].steps.len(), 1, "everything after `a#0` is cached");
    assert_eq!(out[0].steps[0].span, "a#0");
}

#[test]
fn a_session_with_nothing_to_keep_is_not_run_at_all() {
    let out = sessions(
        &[
            beat("a#0", "terminal", "wait 1s"),
            beat("b#0", "terminal", "wait 1s"),
        ],
        &cached(&["a#0", "b#0"]),
    );

    assert!(out.is_empty(), "a warm scene is not a terminal to open");
}

/// A pause holds whatever is on screen. There is nothing to run for it and
/// nothing to keep, and putting it in a session would have a backend try
/// to run a duration as if it were a tape.
#[test]
fn a_pause_is_not_captured() {
    let out = sessions(
        &[
            beat("a#0", "terminal", "wait 1s"),
            beat("pause-1", "pause", "2000"),
            beat("b#0", "terminal", "wait 1s"),
        ],
        &cold,
    );

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].steps.len(), 2);
    assert!(out[0].steps.iter().all(|s| s.span != "pause-1"));
}
