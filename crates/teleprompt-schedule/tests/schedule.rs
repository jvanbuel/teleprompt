use teleprompt_core::config::{Config, TransitionDuration};
use teleprompt_core::Hash;
use teleprompt_core::VoiceSource;
use teleprompt_schedule::{schedule, ActionInput, DurationSource, Item, NarrationInput, Policy};

fn narration(id: &str, ms: u64) -> NarrationInput {
    let defaults = Config::default().timing;
    NarrationInput {
        line_id: id.into(),
        source_hash: Hash::of(id.as_bytes()),
        audio_hash: Hash::of(id.as_bytes()),
        duration_ms: ms,
        duration_source: DurationSource::Measured,
        lead_in_ms: defaults.lead_in_ms,
        tail_ms: defaults.tail_ms,
        voice_source: VoiceSource::Synthetic,
        voice_source_actual: VoiceSource::Synthetic,
        downgrade_reason: None,
    }
}

fn action(id: &str, ms: u64) -> ActionInput {
    ActionInput {
        shot_id: id.into(),
        scene: "mock".into(),
        adapter: "mock".into(),
        shot_hash: Hash::of(id.as_bytes()),
        duration_ms: ms,
        duration_source: DurationSource::Exact,
        cue_ms: None,
        session: None,
    }
}

fn item(id: &str, n: Option<u64>, a: Option<u64>, policy: Policy, cfg: Config) -> Item {
    Item {
        id: id.into(),
        narration: n.map(|ms| narration(id, ms)),
        action: a.map(|ms| action(id, ms)),
        policy,
        config: cfg,
    }
}

/// A hold item carrying only narration, with the default transition config —
/// the shape a script of plain paragraphs produces.
fn narration_item(id: &str, ms: u64) -> Item {
    item(id, Some(ms), None, Policy::Hold, Config::default())
}

fn no_transition() -> Config {
    let mut c = Config::default();
    c.transition.duration = TransitionDuration::Fixed(0);
    c
}

#[test]
fn narration_is_padded_with_lead_in_and_tail() {
    let t = schedule(
        &[item("b1", Some(1000), None, Policy::Hold, no_transition())],
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
fn shots_lay_out_sequentially_when_transitions_are_zero() {
    let cfg = no_transition();
    let items = vec![
        item("b1", Some(1000), None, Policy::Hold, cfg.clone()),
        item("b2", Some(2000), None, Policy::Hold, cfg),
    ];
    let t = schedule(&items, "s.md", "en", "0.1.0").0;
    assert_eq!(t.entries[0].start_ms, 0);
    assert_eq!(t.entries[1].start_ms, 1300);
    assert_eq!(t.duration_ms, 1300 + 2300);
}

#[test]
fn transitions_overlap_adjacent_shots() {
    let mut cfg = Config::default();
    cfg.transition.duration = TransitionDuration::Fixed(300);
    let items = vec![
        item("b1", Some(1000), None, Policy::Hold, cfg.clone()),
        item("b2", Some(1000), None, Policy::Hold, cfg),
    ];
    let t = schedule(&items, "s.md", "en", "0.1.0").0;
    assert_eq!(t.entries[0].duration_ms, 1300);
    assert_eq!(t.entries[1].start_ms, 1000, "1300 - 300 overlap");
    assert_eq!(t.duration_ms, 2300, "1300 + 1300 - 300");
}

#[test]
fn the_last_shot_has_no_outgoing_transition() {
    let mut cfg = Config::default();
    cfg.transition.duration = TransitionDuration::Fixed(300);
    let t = schedule(
        &[item("b1", Some(1000), None, Policy::Hold, cfg)],
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
    let items = vec![
        item(
            "b1",
            Some(5000),
            Some(200),
            Policy::Concurrent(teleprompt_schedule::Align::Start),
            cfg.clone(),
        ),
        item("b2", Some(1000), None, Policy::Hold, cfg),
    ];
    let mut items = items;
    // A generous tail so the quiet window (§6.3) is wider than `max_ms` and
    // the maximum is genuinely what binds. With the default 150ms tail the
    // window is 300ms and the cap would bind instead — which is the subject
    // of `the_auto_transition_fills_the_quiet_window_exactly`, not this test.
    items[0].narration.as_mut().unwrap().tail_ms = 1000;

    let t = schedule(&items, "s.md", "en", "0.1.0").0;
    // slack = 6150 padded narration - 200 action = 5950; half is 2975,
    // clamped to max_ms 600. Quiet window is 1000 tail + 150 next lead_in,
    // so the cap does not bind.
    assert_eq!(t.entries[0].transition.duration_ms, 600);
}

#[test]
fn auto_transition_is_zero_when_there_is_no_slack() {
    let cfg = Config::default();
    let items = vec![
        item(
            "b1",
            Some(1000),
            Some(5000),
            Policy::Concurrent(teleprompt_schedule::Align::Start),
            cfg.clone(),
        ),
        item("b2", Some(1000), None, Policy::Hold, cfg),
    ];
    let t = schedule(&items, "s.md", "en", "0.1.0").0;
    assert_eq!(
        t.entries[0].transition.duration_ms, 0,
        "action outlasts narration"
    );
}

#[test]
fn action_offsets_are_absolute_not_shot_relative() {
    let cfg = no_transition();
    let items = vec![
        item("b1", Some(1000), None, Policy::Hold, cfg.clone()),
        item("b2", Some(1000), Some(500), Policy::Hold, cfg),
    ];
    let t = schedule(&items, "s.md", "en", "0.1.0").0;
    let a = t.entries[1].action.as_ref().unwrap();
    assert_eq!(a.start_ms, 1300 + 1300, "item 2 start + narration");
}

#[test]
fn policy_warnings_are_collected_with_the_shot_id() {
    let mut cfg = no_transition();
    cfg.timing.max_stretch = 2.0;
    let t = schedule(
        &[item("b1", Some(10_000), Some(500), Policy::Stretch, cfg)],
        "s.md",
        "en",
        "0.1.0",
    );
    assert!(t.1[0].contains("b1"));
    assert!(t.1[0].contains("max_stretch"));
}

#[test]
fn an_action_only_shot_schedules_with_no_narration() {
    let t = schedule(
        &[item("b1", None, Some(800), Policy::Hold, no_transition())],
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
    let items = vec![item("welcome", Some(1000), Some(500), Policy::Hold, cfg)];
    let t = schedule(&items, "script.md", "nl", "0.1.0").0;
    insta::assert_json_snapshot!(t);
}

/// Spec §6.3, the quiet window. A script of plain paragraphs has no action to
/// subtract, so `slack` is the whole narration and an uncapped `auto`
/// transition runs to `max_ms` — 600 ms against 300 ms of padding, overlapping
/// 300 ms of the outgoing sentence with the incoming one.
///
/// The cap is what makes spec §3's claim true that such a script's narration
/// follows one line after another. Asserted on the timeline, because that
/// is what both the manifest and the renderer read.
#[test]
fn narration_only_shots_never_talk_over_each_other() {
    let items = vec![
        narration_item("one", 6900),
        narration_item("two", 5300),
        narration_item("three", 4900),
    ];
    let (timeline, _) = schedule(&items, "tour.md", "en", "0.1.0");

    let speech: Vec<(u64, u64)> = timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .map(|n| (n.start_ms, n.start_ms + n.duration_ms))
        .collect();
    assert_eq!(speech.len(), 3);

    for pair in speech.windows(2) {
        let (_, ends) = pair[0];
        let (next_starts, _) = pair[1];
        assert!(
            ends <= next_starts,
            "line ending at {ends}ms overlaps the next, which starts at \
             {next_starts}ms — {}ms of two voices at once",
            ends - next_starts
        );
    }
}

/// The same window, from the other side: the transition must still be as long
/// as the silence allows, so a cap does not become "always cut".
#[test]
fn the_auto_transition_fills_the_quiet_window_exactly() {
    let items = vec![narration_item("one", 6900), narration_item("two", 5300)];
    let (timeline, _) = schedule(&items, "tour.md", "en", "0.1.0");

    // tail 150 leaving the first item + lead_in 150 entering the second.
    assert_eq!(timeline.entries[0].transition.duration_ms, 300);
}

/// A fixed duration is the author asking by name, so it is honoured — but a
/// fixed transition wider than the quiet window produces two voices at once,
/// which they should hear about.
#[test]
fn a_fixed_transition_wider_than_the_quiet_window_warns() {
    let mut items = vec![narration_item("one", 6900), narration_item("two", 5300)];
    for b in &mut items {
        b.config.transition.duration = TransitionDuration::Fixed(2000);
    }
    let (timeline, warnings) = schedule(&items, "tour.md", "en", "0.1.0");

    assert_eq!(
        timeline.entries[0].transition.duration_ms, 2000,
        "honoured as written"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("one") && w.contains("overlap")),
        "{warnings:?}"
    );
}

#[test]
fn narration_entries_report_where_their_duration_came_from() {
    let mut items = vec![narration_item("one", 5000), narration_item("two", 3000)];
    items[0].narration.as_mut().unwrap().duration_source = DurationSource::Measured;
    items[1].narration.as_mut().unwrap().duration_source = DurationSource::Estimated;

    let (t, _) = schedule(&items, "s.md", "en", "0.1.0");
    let sources: Vec<&str> = t
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .map(|n| n.duration_source.as_str())
        .collect();

    assert_eq!(
        sources,
        vec!["measured", "estimated"],
        "a reader must be able to tell a measurement from a prediction"
    );
}

/// C1's second half. Validation now stops the `u64::MAX` duration that used
/// to reach here, but the scheduler is handed `u64`s and must not panic on
/// any triple of them — and a plain add does something worse than panic in a
/// release build, where it wraps and produces a short, confident, wrong
/// timeline.
#[test]
fn padded_duration_saturates_instead_of_overflowing() {
    let mut n = narration("huge", u64::MAX);
    n.lead_in_ms = u64::MAX;
    n.tail_ms = u64::MAX;
    assert_eq!(n.padded_duration_ms(), u64::MAX);
}
