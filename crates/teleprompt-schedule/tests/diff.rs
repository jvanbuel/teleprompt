use teleprompt_core::config::{Config, TransitionDuration};
use teleprompt_core::Hash;
use teleprompt_schedule::{
    diff, schedule, ActionInput, Beat, DurationSource, NarrationInput, Policy,
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
