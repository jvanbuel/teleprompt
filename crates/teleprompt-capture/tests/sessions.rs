//! Grouping a timeline's shots into the sessions a backend can run.
//!
//! All of it is arithmetic over a list: nothing here runs a terminal, and
//! the point of that is that the decisions a capture makes — what to run,
//! what to keep, where to stop — are testable without one.

use std::collections::HashSet;

use teleprompt_capture::{sessions, PlannedShot};
use teleprompt_core::Hash;

fn shot(shot: &str, scene: &str, source: &str) -> PlannedShot {
    PlannedShot {
        id: shot.into(),
        scene: scene.into(),
        adapter: "mock".into(),
        session: None,
        key: Hash::of(shot.as_bytes()),
        source: source.into(),
        duration_ms: 1_000,
        settings: Default::default(),
    }
}

/// Nothing is cached.
fn cold(_: &Hash) -> bool {
    false
}

fn cached(shots: &[&str]) -> impl Fn(&Hash) -> bool {
    let keys: HashSet<Hash> = shots.iter().map(|s| Hash::of(s.as_bytes())).collect();
    move |key: &Hash| keys.contains(key)
}

#[test]
fn the_shots_of_a_scene_are_one_session_in_order() {
    let out = sessions(
        &[
            shot("a#0", "terminal", "wait 1s"),
            shot("b#0", "terminal", "wait 2s"),
        ],
        &cold,
    );

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].scene, "terminal");
    assert_eq!(out[0].shots.len(), 2);
    assert_eq!(out[0].shots[0].id, "a#0");
    assert!(out[0].shots.iter().all(|s| s.wanted));
}

#[test]
fn two_scenes_are_two_sessions() {
    let out = sessions(
        &[
            shot("a#0", "terminal", "wait 1s"),
            shot("b#0", "browser", "wait 1s"),
            shot("c#0", "terminal", "wait 1s"),
        ],
        &cold,
    );

    assert_eq!(out.len(), 2);
    assert_eq!(out[0].shots.len(), 2, "the terminal's two shots");
    assert_eq!(out[1].shots.len(), 1);
}

#[test]
fn a_named_session_is_a_session_of_its_own() {
    let mut second = shot("b#0", "terminal", "wait 1s");
    second.session = Some("retry".into());
    let out = sessions(&[shot("a#0", "terminal", "wait 1s"), second], &cold);

    assert_eq!(out.len(), 2);
    assert_eq!(out[1].name.as_deref(), Some("retry"));
}

/// The property that makes a session a session. A shot whose clip is
/// already in hand still runs, because the shot after it opens on the
/// screen it leaves behind — it just is not kept.
#[test]
fn a_cached_step_still_runs_when_something_after_it_is_wanted() {
    let out = sessions(
        &[
            shot("a#0", "terminal", "wait 1s"),
            shot("b#0", "terminal", "wait 1s"),
        ],
        &cached(&["a#0"]),
    );

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].shots.len(), 2, "both shots run");
    assert!(!out[0].shots[0].wanted, "the first one is not kept");
    assert!(out[0].shots[1].wanted);
    assert_eq!(out[0].wanted(), 1);
}

/// And the other end: a shot after the last wanted one leads nowhere
/// anybody is looking. On a tape whose sleeps are real seconds that is the
/// difference between a capture that stops and one that sits there.
#[test]
fn a_session_stops_after_the_last_step_worth_keeping() {
    let out = sessions(
        &[
            shot("a#0", "terminal", "wait 1s"),
            shot("b#0", "terminal", "wait 1s"),
            shot("c#0", "terminal", "wait 1s"),
        ],
        &cached(&["b#0", "c#0"]),
    );

    assert_eq!(out[0].shots.len(), 1, "everything after `a#0` is cached");
    assert_eq!(out[0].shots[0].id, "a#0");
}

#[test]
fn a_session_with_nothing_to_keep_is_not_run_at_all() {
    let out = sessions(
        &[
            shot("a#0", "terminal", "wait 1s"),
            shot("b#0", "terminal", "wait 1s"),
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
            shot("a#0", "terminal", "wait 1s"),
            shot("pause-1", "pause", "2000"),
            shot("b#0", "terminal", "wait 1s"),
        ],
        &cold,
    );

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].shots.len(), 2);
    assert!(out[0].shots.iter().all(|s| s.id != "pause-1"));
}

/// A scene kind this build records, and one it does not. The difference
/// matters to the caller: "nothing here can record that" is worth
/// installing something about, and it is not the same as a backend that
/// tried and failed.
#[test]
fn a_scene_kind_with_no_backend_has_none_rather_than_a_broken_one() {
    use std::path::Path;
    use teleprompt_capture::{
        CaptureBackend, CaptureError, CaptureRegistry, Clip, Frame, Progress, Session,
    };

    struct Fake;
    impl CaptureBackend for Fake {
        fn adapter(&self) -> &'static str {
            "vhs"
        }
        fn capture(
            &self,
            _: &Session,
            _: &Frame,
            _: &Path,
            _: &mut dyn FnMut(Progress),
        ) -> Result<Vec<Clip>, CaptureError> {
            Ok(Vec::new())
        }
    }

    let registry = CaptureRegistry::new().with(Box::new(Fake));
    assert_eq!(
        registry.for_adapter("vhs").map(|b| b.adapter()),
        Some("vhs")
    );
    assert!(registry.for_adapter("playwright").is_none());
}
