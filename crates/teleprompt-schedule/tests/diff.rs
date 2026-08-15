use teleprompt_core::config::{Config, TransitionDuration};
use teleprompt_core::Hash;
use teleprompt_schedule::{
    diff, schedule, ActionInput, Beat, DurationSource, NarrationInput, Policy, TimelineDiff,
};
use teleprompt_voice::VoiceSource;

fn cfg() -> Config {
    let mut c = Config::default();
    c.transition.duration = TransitionDuration::Fixed(0);
    c
}

fn beat(id: &str, narration_ms: u64, source: &str) -> Beat {
    Beat {
        id: id.into(),
        narration: Some(NarrationInput {
            segment_id: id.into(),
            source_hash: Hash::of(source.as_bytes()),
            audio_hash: Hash::of(source.as_bytes()),
            duration_ms: narration_ms,
            voice_source: VoiceSource::Synthetic,
            voice_source_actual: VoiceSource::Synthetic,
            downgrade_reason: None,
        }),
        action: None,
        policy: Policy::Hold,
        config: cfg(),
    }
}

fn stale_beat(id: &str) -> Beat {
    let mut b = beat(id, 1000, id);
    let n = b.narration.as_mut().unwrap();
    n.voice_source = VoiceSource::Recorded;
    n.voice_source_actual = VoiceSource::Cloned;
    n.downgrade_reason = Some("take stale".into());
    b
}

fn timeline(beats: Vec<Beat>) -> teleprompt_schedule::Timeline {
    schedule(&beats, "s.md", "en", "0.1.0").0
}

#[test]
fn identical_timelines_diff_to_nothing() {
    let a = timeline(vec![beat("b1", 1000, "one")]);
    let b = timeline(vec![beat("b1", 1000, "one")]);
    assert!(diff(&a, &b).is_empty());
}

#[test]
fn a_longer_segment_is_reported_with_both_durations() {
    let a = timeline(vec![beat("b1", 4200, "one")]);
    let b = timeline(vec![beat("b1", 5800, "one but longer")]);
    let d = diff(&a, &b);
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].beat, "b1");
    assert_eq!(d.changed[0].before_ms, 4200);
    assert_eq!(d.changed[0].after_ms, 5800);
    assert!(d.changed[0].reason.contains("text edited"));
}

#[test]
fn a_duration_change_without_a_text_change_is_reported_differently() {
    let a = timeline(vec![beat("b1", 4200, "same")]);
    let mut changed = beat("b1", 5000, "same");
    changed.narration.as_mut().unwrap().audio_hash = Hash::of(b"different voice");
    let b = timeline(vec![changed]);
    let d = diff(&a, &b);
    assert!(d.changed[0].reason.contains("audio changed"));
    assert!(!d.changed[0].reason.contains("text edited"));
}

#[test]
fn total_shift_is_the_difference_in_overall_duration() {
    let a = timeline(vec![beat("b1", 4200, "one"), beat("b2", 1000, "two")]);
    let b = timeline(vec![
        beat("b1", 5800, "one longer"),
        beat("b2", 1000, "two"),
    ]);
    let d = diff(&a, &b);
    assert_eq!(d.shift_ms, 1600);
    assert_eq!(d.after_ms - d.before_ms, 1600);
}

#[test]
fn added_and_removed_beats_are_listed_separately() {
    let a = timeline(vec![beat("b1", 1000, "one")]);
    let b = timeline(vec![beat("b1", 1000, "one"), beat("b2", 1000, "two")]);
    let d = diff(&a, &b);
    assert_eq!(d.added, ["b2"]);
    assert!(d.removed.is_empty());

    let back = diff(&b, &a);
    assert_eq!(back.removed, ["b2"]);
    assert!(back.added.is_empty());
}

#[test]
fn stale_takes_are_reported_with_their_fallback_tier() {
    let a = timeline(vec![beat("b1", 1000, "one")]);
    let b = timeline(vec![stale_beat("b1")]);
    let d = diff(&a, &b);
    assert_eq!(d.stale_takes.len(), 1);
    assert_eq!(d.stale_takes[0].segment, "b1");
    assert_eq!(d.stale_takes[0].falls_back_to, "cloned");
}

#[test]
fn beats_needing_recapture_are_those_whose_action_hash_or_slot_changed() {
    let action = |id: &str, ms: u64| ActionInput {
        span_id: id.into(),
        scene: "mock".into(),
        adapter: "mock".into(),
        span_hash: Hash::of(id.as_bytes()),
        duration_ms: ms,
        duration_source: DurationSource::Exact,
    };
    let mut a1 = beat("b1", 1000, "one");
    a1.action = Some(action("s1", 500));
    let mut b1 = beat("b1", 2000, "one longer");
    b1.action = Some(action("s1", 500));

    let d = diff(&timeline(vec![a1]), &timeline(vec![b1]));
    assert_eq!(
        d.recapture,
        ["b1"],
        "the slot moved even though the span did not"
    );
}

/// Review round 1, finding 1: a reworded sentence of identical spoken
/// length must not be invisible to `diff` — its committed `source_hash`
/// disagrees with the script even though the clock didn't move, and a
/// silent diff there would let `--exit-code` report clean over a stale
/// baseline.
#[test]
fn a_text_edit_with_no_duration_change_is_still_reported() {
    let a = timeline(vec![beat("b1", 1000, "one")]);
    let b = timeline(vec![beat("b1", 1000, "one but reworded")]);
    let d = diff(&a, &b);

    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].beat, "b1");
    assert_eq!(d.changed[0].before_ms, 1000);
    assert_eq!(d.changed[0].after_ms, 1000);
    assert!(d.changed[0].reason.contains("text edited"));
    assert!(!d.is_empty());

    let rendered = d.render();
    assert!(rendered.contains("b1"));
    assert!(rendered.contains("text edited"));
    assert!(
        !rendered.contains("1.0s \u{2192} 1.0s"),
        "an unchanged duration must not render as a no-op arrow: {rendered:?}"
    );
}

fn mock_action(id: &str, ms: u64) -> ActionInput {
    ActionInput {
        span_id: id.into(),
        scene: "mock".into(),
        adapter: "mock".into(),
        span_hash: Hash::of(id.as_bytes()),
        duration_ms: ms,
        duration_source: DurationSource::Exact,
    }
}

/// Review round 1, finding 2: a beat's slot changed just as much by
/// gaining or losing an action as by an existing action's span or timing
/// changing.
#[test]
fn a_beat_that_gains_an_action_needs_recapture() {
    let a = timeline(vec![beat("b1", 1000, "one")]);
    let mut gained = beat("b1", 1000, "one");
    gained.action = Some(mock_action("s1", 500));
    let b = timeline(vec![gained]);

    let d = diff(&a, &b);
    assert_eq!(d.recapture, ["b1"]);
}

#[test]
fn a_beat_that_loses_an_action_needs_recapture() {
    let mut had_action = beat("b1", 1000, "one");
    had_action.action = Some(mock_action("s1", 500));
    let a = timeline(vec![had_action]);
    let b = timeline(vec![beat("b1", 1000, "one")]);

    let d = diff(&a, &b);
    assert_eq!(d.recapture, ["b1"]);
}

#[test]
fn rendered_output_names_the_script_change_and_the_shift() {
    let a = timeline(vec![
        beat("welcome", 4200, "one"),
        beat("next", 1000, "two"),
    ]);
    let b = timeline(vec![
        beat("welcome", 5800, "one longer"),
        beat("next", 1000, "two"),
    ]);
    let rendered = diff(&a, &b).render();
    assert!(rendered.contains("welcome"));
    assert!(rendered.contains("4.2s"));
    assert!(rendered.contains("5.8s"));
    assert!(rendered.contains("+1.6s"));
}

#[test]
fn an_empty_diff_renders_a_single_reassuring_line() {
    let a = timeline(vec![beat("b1", 1000, "one")]);
    assert_eq!(diff(&a, &a).render(), "no timeline changes");
}

/// Not in the brief, but load-bearing for Task 14: a committed timeline is
/// written to disk and read back, so `Serialize`/`Deserialize` must be
/// exact inverses — including for the optional fields (`action`,
/// `downgrade_reason`) that are skipped on write and so must default to
/// `None` on read rather than failing to deserialize.
#[test]
fn a_timeline_with_no_action_and_no_downgrade_reason_round_trips_through_json() {
    let original = timeline(vec![beat("b1", 1000, "one")]);
    let json = serde_json::to_string_pretty(&original).expect("serialize");
    let restored: teleprompt_schedule::Timeline = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored, original);
}

/// Task 14 review round 1, finding 1: the header's three printed numbers
/// must reconcile. Rounding `before_ms`, `after_ms`, and the raw
/// `shift_ms` independently to one decimal is not guaranteed to agree,
/// because tenths-of-a-second rounding is not linear: 1ms rounds to 0.0s,
/// 50ms rounds to 0.1s (round-half-away-from-zero), but the exact 49ms
/// difference between them also rounds to 0.0s on its own. A renderer that
/// rounds all three independently would print "0.0s → 0.1s (+0.0s)",
/// which visibly doesn't add up. The displayed shift must instead be
/// derived from the two already-rounded endpoints.
#[test]
fn rendered_shift_reconciles_with_the_rounded_endpoints() {
    let d = TimelineDiff {
        before_ms: 1,
        after_ms: 50,
        shift_ms: 49,
        changed: vec![],
        added: vec!["b1".to_string()],
        removed: vec![],
        stale_takes: vec![],
        recapture: vec![],
    };

    let rendered = d.render();
    assert!(
        rendered.contains("0.0s \u{2192} 0.1s (+0.1s)"),
        "the printed shift must equal the printed after minus the printed before: {rendered:?}"
    );

    // The struct (and its JSON) must keep the exact millisecond shift —
    // only the prose header is corrected, not the underlying data, so a
    // future refactor can't "fix" this by rounding what's stored.
    assert_eq!(d.shift_ms, 49);
}
