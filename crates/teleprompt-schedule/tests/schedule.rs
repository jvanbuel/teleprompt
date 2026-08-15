use teleprompt_core::config::{Config, TransitionDuration};
use teleprompt_core::Hash;
use teleprompt_schedule::{schedule, ActionInput, Beat, DurationSource, NarrationInput, Policy};
use teleprompt_voice::VoiceSource;

fn narration(id: &str, ms: u64) -> NarrationInput {
    NarrationInput {
        segment_id: id.into(),
        source_hash: Hash::of(id.as_bytes()),
        audio_hash: Hash::of(id.as_bytes()),
        duration_ms: ms,
        voice_source: VoiceSource::Synthetic,
        voice_source_actual: VoiceSource::Synthetic,
        downgrade_reason: None,
    }
}

fn action(id: &str, ms: u64) -> ActionInput {
    ActionInput {
        span_id: id.into(),
        scene: "mock".into(),
        adapter: "mock".into(),
        span_hash: Hash::of(id.as_bytes()),
        duration_ms: ms,
        duration_source: DurationSource::Exact,
    }
}

fn beat(id: &str, n: Option<u64>, a: Option<u64>, policy: Policy, cfg: Config) -> Beat {
    Beat {
        id: id.into(),
        narration: n.map(|ms| narration(id, ms)),
        action: a.map(|ms| action(id, ms)),
        policy,
        config: cfg,
    }
}

fn no_transition() -> Config {
    let mut c = Config::default();
    c.transition.duration = TransitionDuration::Fixed(0);
    c
}

#[test]
fn narration_is_padded_with_lead_in_and_tail() {
    let t = schedule(
        &[beat("b1", Some(1000), None, Policy::Hold, no_transition())],
        "s.md",
        "en",
        "0.1.0",
    )
    .0;
    // 150 lead-in + 1000 clip + 150 tail
    assert_eq!(t.entries[0].duration_ms, 1300);
    assert_eq!(t.entries[0].narration.as_ref().unwrap().duration_ms, 1000);
}

#[test]
fn beats_lay_out_sequentially_when_transitions_are_zero() {
    let cfg = no_transition();
    let beats = vec![
        beat("b1", Some(1000), None, Policy::Hold, cfg.clone()),
        beat("b2", Some(2000), None, Policy::Hold, cfg),
    ];
    let t = schedule(&beats, "s.md", "en", "0.1.0").0;
    assert_eq!(t.entries[0].start_ms, 0);
    assert_eq!(t.entries[1].start_ms, 1300);
    assert_eq!(t.duration_ms, 1300 + 2300);
}

#[test]
fn transitions_overlap_adjacent_beats() {
    let mut cfg = Config::default();
    cfg.transition.duration = TransitionDuration::Fixed(300);
    let beats = vec![
        beat("b1", Some(1000), None, Policy::Hold, cfg.clone()),
        beat("b2", Some(1000), None, Policy::Hold, cfg),
    ];
    let t = schedule(&beats, "s.md", "en", "0.1.0").0;
    assert_eq!(t.entries[0].duration_ms, 1300);
    assert_eq!(t.entries[1].start_ms, 1000, "1300 - 300 overlap");
    assert_eq!(t.duration_ms, 2300, "1300 + 1300 - 300");
}

#[test]
fn the_last_beat_has_no_outgoing_transition() {
    let mut cfg = Config::default();
    cfg.transition.duration = TransitionDuration::Fixed(300);
    let t = schedule(
        &[beat("b1", Some(1000), None, Policy::Hold, cfg)],
        "s.md",
        "en",
        "0.1.0",
    )
    .0;
    assert_eq!(t.entries[0].transition.duration_ms, 0);
}

#[test]
fn auto_transition_is_half_the_slack_clamped_to_the_maximum() {
    let cfg = Config::default(); // auto, min 0, max 600
    let beats = vec![
        beat(
            "b1",
            Some(5000),
            Some(200),
            Policy::Concurrent(teleprompt_schedule::Align::Start),
            cfg.clone(),
        ),
        beat("b2", Some(1000), None, Policy::Hold, cfg),
    ];
    let t = schedule(&beats, "s.md", "en", "0.1.0").0;
    // slack = 5300 padded narration - 200 action = 5100; half is 2550, clamped to 600
    assert_eq!(t.entries[0].transition.duration_ms, 600);
}

#[test]
fn auto_transition_is_zero_when_there_is_no_slack() {
    let cfg = Config::default();
    let beats = vec![
        beat(
            "b1",
            Some(1000),
            Some(5000),
            Policy::Concurrent(teleprompt_schedule::Align::Start),
            cfg.clone(),
        ),
        beat("b2", Some(1000), None, Policy::Hold, cfg),
    ];
    let t = schedule(&beats, "s.md", "en", "0.1.0").0;
    assert_eq!(
        t.entries[0].transition.duration_ms, 0,
        "action outlasts narration"
    );
}

#[test]
fn action_offsets_are_absolute_not_beat_relative() {
    let cfg = no_transition();
    let beats = vec![
        beat("b1", Some(1000), None, Policy::Hold, cfg.clone()),
        beat("b2", Some(1000), Some(500), Policy::Hold, cfg),
    ];
    let t = schedule(&beats, "s.md", "en", "0.1.0").0;
    let a = t.entries[1].action.as_ref().unwrap();
    assert_eq!(a.start_ms, 1300 + 1300, "beat 2 start + narration");
}

#[test]
fn policy_warnings_are_collected_with_the_beat_id() {
    let mut cfg = no_transition();
    cfg.timing.max_stretch = 2.0;
    let t = schedule(
        &[beat("b1", Some(10_000), Some(500), Policy::Stretch, cfg)],
        "s.md",
        "en",
        "0.1.0",
    );
    assert!(t.1[0].contains("b1"));
    assert!(t.1[0].contains("max_stretch"));
}

#[test]
fn an_action_only_beat_schedules_with_no_narration() {
    let t = schedule(
        &[beat("b1", None, Some(800), Policy::Hold, no_transition())],
        "s.md",
        "en",
        "0.1.0",
    )
    .0;
    assert!(t.entries[0].narration.is_none());
    assert_eq!(t.entries[0].duration_ms, 800);
}

#[test]
fn an_empty_program_produces_an_empty_timeline() {
    let t = schedule(&[], "s.md", "en", "0.1.0").0;
    assert_eq!(t.duration_ms, 0);
    assert!(t.entries.is_empty());
}

#[test]
fn the_timeline_json_shape_is_stable() {
    let cfg = no_transition();
    let beats = vec![beat("welcome", Some(1000), Some(500), Policy::Hold, cfg)];
    let t = schedule(&beats, "script.md", "nl", "0.1.0").0;
    insta::assert_json_snapshot!(t);
}
