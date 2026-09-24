use teleprompt_core::config::{Config, TransitionDuration};
use teleprompt_core::DurationMs;
use teleprompt_core::Hash;
use teleprompt_core::{DurationSource, VoiceSource};
use teleprompt_schedule::{schedule, ActionInput, Item, NarrationInput, Pacing, Policy};

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
        pacing: Pacing::from(&cfg),
    }
}

/// A hold item carrying only narration, with the default transition config —
/// the shape a script of plain paragraphs produces.
fn narration_item(id: &str, ms: u64) -> Item {
    item(id, Some(ms), None, Policy::Hold, Config::default())
}

fn no_transition() -> Config {
    let mut c = Config::default();
    c.transition.duration = TransitionDuration::Fixed(DurationMs::millis(0));
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
    cfg.transition.duration = TransitionDuration::Fixed(DurationMs::millis(300));
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
    cfg.transition.duration = TransitionDuration::Fixed(DurationMs::millis(300));
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
            Policy::Concurrent(teleprompt_core::policy::Align::Start),
            cfg.clone(),
        ),
        item("b2", Some(1000), None, Policy::Hold, cfg),
    ];
    let mut items = items;
    // A generous tail so the quiet window is wider than `max_ms` and the
    // maximum is genuinely what binds. With the default 150ms tail the window
    // is 300ms and the cap would bind instead — which is the subject of
    // `the_auto_transition_fills_the_quiet_window_exactly`, not this test.
    items[0].narration.as_mut().unwrap().tail_ms = DurationMs::millis(1000);

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
            Policy::Concurrent(teleprompt_core::policy::Align::Start),
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

/// The quiet window (docs/design.md#quiet-window). A script of plain paragraphs
/// has no action to subtract, so `slack` is the whole narration and an uncapped
/// `auto` transition runs to `max_ms` — 600 ms against 300 ms of padding,
/// overlapping 300 ms of the outgoing sentence with the incoming one.
///
/// The cap is what makes such a script's narration follow one line after
/// another. Asserted on the timeline, because that is what both the manifest
/// and the renderer read.
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
        b.pacing.transition.duration = TransitionDuration::Fixed(DurationMs::millis(2000));
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
        .map(|n| n.duration_source.label())
        .collect();

    assert_eq!(
        sources,
        vec!["measured", "estimated"],
        "a reader must be able to tell a measurement from a prediction"
    );
}

/// Lead-in and tail are bounded by their type, but the clip is measured and
/// is not, so padding it must saturate: a plain add wraps in a release build
/// and produces a short, confident, wrong timeline.
#[test]
fn padded_duration_saturates_instead_of_overflowing() {
    let mut n = narration("huge", u64::MAX);
    n.lead_in_ms = DurationMs::MAX;
    n.tail_ms = DurationMs::MAX;
    assert_eq!(n.padded_duration_ms(), u64::MAX);
}

/// A transition may not consume what the transition before it already took.
///
/// The scheduler capped each transition at the item it leaves, which is not
/// enough: an item can be long enough for the transition arriving into it
/// *and* for the one leaving it, and still too short for both together.
/// When that happened the two blends drew the same frames twice, the
/// incremental renderer could not cut the plan into chunks at all, and it
/// silently handed the whole build to a second renderer.
///
/// Scheduling the constraint instead of detecting it downstream means every
/// plan can be chunked, which is what lets there be one renderer.
#[test]
fn a_transition_leaves_room_for_the_one_that_arrived_before_it() {
    let mut cfg = Config::default();
    cfg.transition.duration = TransitionDuration::Fixed(DurationMs::millis(400));
    cfg.transition.min_ms = DurationMs::millis(0);

    // A short item between two long ones: 400ms in, 400ms out, 500ms of item.
    let items = vec![
        item("a", Some(4000), None, Policy::Hold, cfg.clone()),
        item("b", Some(200), None, Policy::Hold, cfg.clone()),
        item("c", Some(4000), None, Policy::Hold, cfg.clone()),
    ];
    let t = schedule(&items, "s.md", "en", "0.1.0").0;

    let entries = &t.entries;
    let into_b = entries[0].transition.duration_ms;
    let out_of_b = entries[1].transition.duration_ms;

    assert!(
        into_b + out_of_b <= entries[1].duration_ms,
        "b is {}ms and hosts {into_b}ms in and {out_of_b}ms out; the two \
         transitions overlap each other",
        entries[1].duration_ms
    );
}

/// An action whose adapter cannot say how long it takes fills the slot the
/// narration gives it.
///
/// A Playwright script states no duration — `await page.click(…)` takes as
/// long as the page takes — so its adapter answers `Unknown`. That was
/// flattened to `0`, which is not "unknown", it is "takes no time": the
/// shot got zero length, its picture never reached the video, and the
/// renderer held the neighbouring frame over the whole slot. The build
/// reported `slates: 0, captured: 2` while showing neither.
///
/// Unknown means the recorder is told how long to keep going, which is the
/// only useful reading: the script runs under the sentence, and the
/// sentence decides when the shot is over.
#[test]
fn an_action_of_unknown_length_fills_the_sentence_over_it() {
    let mut item = item("a", Some(6_000), Some(0), Policy::Hold, no_transition());
    if let Some(action) = item.action.as_mut() {
        action.duration_source = DurationSource::Unknown;
    }
    let t = schedule(&[item], "s.md", "en", "0.1.0").0;

    let action = t.entries[0].action.as_ref().expect("an action");
    // 6300, not 6000: the slot is the *padded* narration — lead-in and
    // tail included — because the picture has to cover the silence either
    // side of the speech as well as the speech.
    assert_eq!(
        action.duration_ms, 6_300,
        "an unknown-length action takes the slot the narration gives it, not zero"
    );
}

/// A cue is measured from the narration's first word, which is said one
/// lead-in after the item begins. Placing the action at the bare offset
/// started every cued action a lead-in early — 150 ms by default — which
/// went unseen while cues were interpolated and was plain once they came
/// from word timings.
#[test]
fn a_cue_lands_on_the_word_not_a_lead_in_before_it() {
    let mut it = item(
        "b1",
        Some(3000),
        Some(500),
        Policy::Concurrent(teleprompt_core::policy::Align::Start),
        no_transition(),
    );
    it.action.as_mut().unwrap().cue_ms = Some(1000);
    let lead_in = it.narration.as_ref().unwrap().lead_in_ms.ms();
    let t = schedule(&[it], "s.md", "en", "0.1.0").0;
    assert_eq!(
        t.entries[0].action.as_ref().unwrap().start_ms,
        lead_in + 1000
    );
}

/// No input panics or wraps (#22): what an author wrote is at most a day by
/// type, and the measured and adapter-reported values that are not bounded
/// saturate, so offsets only ever move forward.
#[test]
fn absurd_durations_saturate_rather_than_overflow() {
    let mut held = item(
        "held",
        Some(1000),
        Some(u64::MAX),
        Policy::Hold,
        Config::default(),
    );
    held.narration.as_mut().unwrap().lead_in_ms = DurationMs::MAX;
    let mut cued = item(
        "cued",
        Some(1000),
        Some(1000),
        Policy::Concurrent(teleprompt_core::policy::Align::Start),
        Config::default(),
    );
    cued.narration.as_mut().unwrap().lead_in_ms = DurationMs::MAX;
    cued.action.as_mut().unwrap().cue_ms = Some(u64::MAX);
    let after = item("after", Some(1000), None, Policy::Hold, Config::default());

    for items in [vec![held, after.clone()], vec![cued, after]] {
        let (t, _) = schedule(&items, "s.md", "en", "0.1.0");
        let starts: Vec<u64> = t.entries.iter().map(|e| e.start_ms).collect();
        assert!(starts.windows(2).all(|w| w[0] <= w[1]), "{starts:?}");
        assert!(t.entries.iter().all(|e| e.start_ms <= t.duration_ms));
    }
}
