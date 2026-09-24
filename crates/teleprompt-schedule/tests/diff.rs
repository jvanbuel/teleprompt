use teleprompt_core::config::{Config, TransitionDuration};
use teleprompt_core::DurationMs;
use teleprompt_core::Hash;
use teleprompt_core::{DurationSource, VoiceSource, VoiceTier};
use teleprompt_schedule::{
    diff, schedule, ActionInput, ChangeReason, Item, NarrationInput, Pacing, Policy, TimelineDiff,
};

fn cfg() -> Config {
    let mut c = Config::default();
    c.transition.duration = TransitionDuration::Fixed(DurationMs::millis(0));
    c
}

fn item(id: &str, narration_ms: u64, source: &str) -> Item {
    Item {
        id: id.into(),
        narration: Some(NarrationInput {
            line_id: id.into(),
            source_hash: Hash::of(source.as_bytes()),
            audio_hash: Hash::of(source.as_bytes()),
            duration_ms: narration_ms,
            duration_source: DurationSource::Measured,
            lead_in_ms: Config::default().timing.lead_in_ms,
            tail_ms: Config::default().timing.tail_ms,
            voice: VoiceTier::delivered(VoiceSource::Synthetic),
        }),
        action: None,
        policy: Policy::Hold,
        pacing: Pacing::from(&cfg()),
    }
}

fn stale_shot(id: &str) -> Item {
    let mut b = item(id, 1000, id);
    let n = b.narration.as_mut().unwrap();
    n.voice = VoiceTier::downgraded(
        VoiceSource::Recorded,
        VoiceSource::Cloned,
        "take stale".into(),
    )
    .unwrap();
    b
}

fn timeline(items: Vec<Item>) -> teleprompt_schedule::Timeline {
    schedule(&items, "s.md", "en", "0.1.0").0
}

#[test]
fn identical_timelines_diff_to_nothing() {
    let a = timeline(vec![item("b1", 1000, "one")]);
    let b = timeline(vec![item("b1", 1000, "one")]);
    assert!(diff(&a, &b).is_empty());
}

#[test]
fn a_longer_line_is_reported_with_both_durations() {
    let a = timeline(vec![item("b1", 4200, "one")]);
    let b = timeline(vec![item("b1", 5800, "one but longer")]);
    let d = diff(&a, &b);
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].item, "b1");
    assert_eq!(d.changed[0].before_ms, 4200);
    assert_eq!(d.changed[0].after_ms, 5800);
    assert_eq!(d.changed[0].reason, ChangeReason::TextEdited);
}

#[test]
fn a_duration_change_without_a_text_change_is_reported_differently() {
    let a = timeline(vec![item("b1", 4200, "same")]);
    let mut changed = item("b1", 5000, "same");
    changed.narration.as_mut().unwrap().audio_hash = Hash::of(b"different voice");
    let b = timeline(vec![changed]);
    let d = diff(&a, &b);
    assert_eq!(d.changed[0].reason, ChangeReason::AudioChanged);
    assert_ne!(d.changed[0].reason, ChangeReason::TextEdited);
}

#[test]
fn total_shift_is_the_difference_in_overall_duration() {
    let a = timeline(vec![item("b1", 4200, "one"), item("b2", 1000, "two")]);
    let b = timeline(vec![
        item("b1", 5800, "one longer"),
        item("b2", 1000, "two"),
    ]);
    let d = diff(&a, &b);
    assert_eq!(d.shift_ms, 1600);
    assert_eq!(d.after_ms - d.before_ms, 1600);
}

#[test]
fn added_and_removed_shots_are_listed_separately() {
    let a = timeline(vec![item("b1", 1000, "one")]);
    let b = timeline(vec![item("b1", 1000, "one"), item("b2", 1000, "two")]);
    let d = diff(&a, &b);
    assert_eq!(d.added, ["b2"]);
    assert!(d.removed.is_empty());

    let back = diff(&b, &a);
    assert_eq!(back.removed, ["b2"]);
    assert!(back.added.is_empty());
}

#[test]
fn stale_takes_are_reported_with_their_fallback_tier() {
    let a = timeline(vec![item("b1", 1000, "one")]);
    let b = timeline(vec![stale_shot("b1")]);
    let d = diff(&a, &b);
    assert_eq!(d.stale_takes.len(), 1);
    assert_eq!(d.stale_takes[0].line, "b1");
    assert_eq!(d.stale_takes[0].falls_back_to, VoiceSource::Cloned);
}

#[test]
fn shots_needing_recapture_are_those_whose_action_hash_or_slot_changed() {
    let action = |id: &str, ms: u64| ActionInput {
        shot_id: id.into(),
        scene: "mock".into(),
        adapter: "mock".into(),
        shot_hash: Hash::of(id.as_bytes()),
        duration_ms: ms,
        duration_source: DurationSource::Exact,
        cue_ms: None,
        session: None,
    };
    let mut a1 = item("b1", 1000, "one");
    a1.action = Some(action("s1", 500));
    let mut b1 = item("b1", 2000, "one longer");
    b1.action = Some(action("s1", 500));

    let d = diff(&timeline(vec![a1]), &timeline(vec![b1]));
    assert_eq!(
        d.recapture,
        ["b1"],
        "the slot moved even though the shot did not"
    );
}

/// Review round 1, finding 1: a reworded sentence of identical spoken
/// length must not be invisible to `diff` — its committed `source_hash`
/// disagrees with the script even though the clock didn't move, and a
/// silent diff there would let `--exit-code` report clean over a stale
/// baseline.
#[test]
fn a_text_edit_with_no_duration_change_is_still_reported() {
    let a = timeline(vec![item("b1", 1000, "one")]);
    let b = timeline(vec![item("b1", 1000, "one but reworded")]);
    let d = diff(&a, &b);

    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].item, "b1");
    assert_eq!(d.changed[0].before_ms, 1000);
    assert_eq!(d.changed[0].after_ms, 1000);
    assert_eq!(d.changed[0].reason, ChangeReason::TextEdited);
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

/// Review round 1, finding 2: an item's slot changed just as much by
/// gaining or losing an action as by an existing action's shot or timing
/// changing.
#[test]
fn a_shot_that_gains_an_action_needs_recapture() {
    let a = timeline(vec![item("b1", 1000, "one")]);
    let mut gained = item("b1", 1000, "one");
    gained.action = Some(mock_action("s1", 500));
    let b = timeline(vec![gained]);

    let d = diff(&a, &b);
    assert_eq!(d.recapture, ["b1"]);
}

#[test]
fn a_shot_that_loses_an_action_needs_recapture() {
    let mut had_action = item("b1", 1000, "one");
    had_action.action = Some(mock_action("s1", 500));
    let a = timeline(vec![had_action]);
    let b = timeline(vec![item("b1", 1000, "one")]);

    let d = diff(&a, &b);
    assert_eq!(d.recapture, ["b1"]);
}

#[test]
fn rendered_output_names_the_script_change_and_the_shift() {
    let a = timeline(vec![
        item("welcome", 4200, "one"),
        item("next", 1000, "two"),
    ]);
    let b = timeline(vec![
        item("welcome", 5800, "one longer"),
        item("next", 1000, "two"),
    ]);
    let rendered = diff(&a, &b).render();
    assert!(rendered.contains("welcome"));
    assert!(rendered.contains("4.2s"));
    assert!(rendered.contains("5.8s"));
    assert!(rendered.contains("+1.6s"));
}

#[test]
fn an_empty_diff_renders_a_single_reassuring_line() {
    let a = timeline(vec![item("b1", 1000, "one")]);
    assert_eq!(diff(&a, &a).render(), "no timeline changes");
}

/// A committed timeline is written to disk and read back, so
/// `Serialize`/`Deserialize` must be exact inverses — including for the
/// optional fields (`action`, `downgrade_reason`) that are skipped on write and
/// so must default to `None` on read rather than failing to deserialize.
#[test]
fn a_timeline_with_no_action_and_no_downgrade_reason_round_trips_through_json() {
    let original = timeline(vec![item("b1", 1000, "one")]);
    let json = serde_json::to_string_pretty(&original).expect("serialize");
    let restored: teleprompt_schedule::Timeline = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored, original);
}

/// The header's three printed numbers must reconcile. Rounding `before_ms`,
/// `after_ms`, and the raw `shift_ms` independently to one decimal is not
/// guaranteed to agree, because tenths-of-a-second rounding is not linear: 1ms
/// rounds to 0.0s, 50ms rounds to 0.1s (round-half-away-from-zero), but the
/// exact 49ms difference between them also rounds to 0.0s on its own. A
/// renderer that rounds all three independently would print "0.0s → 0.1s
/// (+0.0s)", which visibly doesn't add up. The displayed shift must instead be
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
        reordered: vec![],
        transitions: vec![],
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

// ---------------------------------------------------------------------------
// `diff` was blind to whole classes of drift.
//
// Every test above this line builds its items with
// `TransitionDuration::Fixed(0)` and none of them reorders items, which is
// why 195 green tests said nothing about any of the following.
// ---------------------------------------------------------------------------

/// Guard: the fix must not make an unchanged timeline look changed. This is
/// the property every one of the checks below is traded against, so it is
/// asserted on the exact rendered string, not on `is_empty()` alone.
#[test]
fn two_identical_timelines_still_render_exactly_no_timeline_changes() {
    let build = || {
        let mut auto = Config::default();
        auto.transition.duration = TransitionDuration::Auto;
        let mut b1 = item("b1", 4000, "one");
        b1.pacing = Pacing::from(&auto);
        b1.action = Some(ActionInput {
            shot_id: "s1".into(),
            scene: "mock".into(),
            adapter: "mock".into(),
            shot_hash: Hash::of(b"s1"),
            duration_ms: 500,
            duration_source: DurationSource::Exact,
            cue_ms: None,
            session: None,
        });
        let mut b2 = item("b2", 2000, "two");
        b2.pacing = Pacing::from(&auto);
        timeline(vec![b1, b2])
    };
    let d = diff(&build(), &build());
    assert!(d.is_empty());
    assert_eq!(d.render(), "no timeline changes");
}

/// Vector one, reproduced against the real binary as
/// `output.transition.max_ms: 600 -> 0`: every gap between items changes,
/// the total moves, and yet not one per-item hash or duration differs.
/// Before the fix this printed `no timeline changes` and exited 0.
#[test]
fn retuning_the_transition_budget_is_reported_and_is_not_empty() {
    let build = |max_ms: u64| {
        let mut c = Config::default();
        c.transition.duration = TransitionDuration::Auto;
        c.transition.max_ms = DurationMs::millis(max_ms);
        let mut b1 = item("b1", 4000, "one");
        b1.pacing = Pacing::from(&c);
        b1.action = Some(ActionInput {
            shot_id: "s1".into(),
            scene: "mock".into(),
            adapter: "mock".into(),
            shot_hash: Hash::of(b"s1"),
            duration_ms: 500,
            duration_source: DurationSource::Exact,
            cue_ms: None,
            session: None,
        });
        b1.policy = Policy::Concurrent(teleprompt_core::policy::Align::Start);
        let mut b2 = item("b2", 2000, "two");
        b2.pacing = Pacing::from(&c);
        timeline(vec![b1, b2])
    };
    let before = build(600);
    let after = build(0);
    assert_ne!(
        before.duration_ms, after.duration_ms,
        "the fixture must actually move the total, or it proves nothing"
    );

    let d = diff(&before, &after);
    assert!(!d.is_empty(), "a re-timed timeline is not a clean diff");
    assert_eq!(d.transitions.len(), 1, "only b1 has an outgoing transition");
    assert_eq!(d.transitions[0].item, "b1");
    // 300, not the configured 600: the quiet window caps an `auto` transition
    // at the quiet window, which here is this item's 150ms tail plus the next
    // item's 150ms lead_in. What this test guards is unaffected — retuning
    // `max_ms` still moves every gap and the total while no item's own hash or
    // duration changes, which is what `diff` used to miss.
    assert_eq!(d.transitions[0].before_ms, 300);
    assert_eq!(d.transitions[0].after_ms, 0);
    assert!(d.changed.is_empty(), "no item's own content changed");

    let rendered = d.render();
    assert!(rendered.contains("transitions:"), "{rendered}");
    assert!(rendered.contains("b1"), "{rendered}");
}

/// Vector two: two paragraphs that both carry an explicit `{#id}` swap
/// places. Nothing is added, nothing is removed, no item's content changes —
/// only the order the video plays them in. Before the fix this printed
/// `no timeline changes`.
#[test]
fn swapping_two_shots_is_reported_as_reordering_not_as_add_remove() {
    let before = timeline(vec![item("alpha", 1000, "one"), item("beta", 2000, "two")]);
    let after = timeline(vec![item("beta", 2000, "two"), item("alpha", 1000, "one")]);

    let d = diff(&before, &after);
    assert!(!d.is_empty(), "a reordered timeline is not a clean diff");
    assert!(d.added.is_empty(), "a reordered item was not added");
    assert!(d.removed.is_empty(), "a reordered item was not removed");
    assert_eq!(
        d.reordered.len(),
        1,
        "a two-item swap names one moved item, not both: {:?}",
        d.reordered
    );
    assert_ne!(
        d.reordered[0].before_index, d.reordered[0].after_index,
        "a reported item must actually have moved"
    );

    let rendered = d.render();
    assert!(rendered.contains("reordered:"), "{rendered}");
    assert!(rendered.contains("position"), "{rendered}");
}

/// Adding an item in the middle shifts every later item's index, but nothing
/// was reordered — the surviving items still play in the same relative
/// order. Reporting them would turn every insertion into a wall of noise.
#[test]
fn inserting_a_shot_shifts_indices_without_reporting_a_reorder() {
    let before = timeline(vec![item("b1", 1000, "one"), item("b3", 1000, "three")]);
    let after = timeline(vec![
        item("b1", 1000, "one"),
        item("b2", 1000, "two"),
        item("b3", 1000, "three"),
    ]);

    let d = diff(&before, &after);
    assert_eq!(d.added, ["b2"]);
    assert!(
        d.reordered.is_empty(),
        "an insertion is not a reorder: {:?}",
        d.reordered
    );
}

/// A total-duration change with no other observable cause must still fail
/// `--exit-code`. `before_ms`/`after_ms`/`shift_ms` are independent facts,
/// not summaries of `changed`.
#[test]
fn a_bare_total_duration_change_is_not_an_empty_diff() {
    let d = TimelineDiff {
        before_ms: 7150,
        after_ms: 8350,
        shift_ms: 1200,
        changed: vec![],
        added: vec![],
        removed: vec![],
        reordered: vec![],
        transitions: vec![],
        stale_takes: vec![],
        recapture: vec![],
    };
    assert!(!d.is_empty());
    assert!(d.render().contains("7.2s \u{2192} 8.4s"), "{}", d.render());
}

/// The narration's placement *within its own item* is a local fact: a
/// line-level `lead_in=` retune that leaves the item's overall length
/// alone still moves the audio, and `diff` must say so.
#[test]
fn a_lead_in_retune_that_does_not_change_shot_length_is_still_reported() {
    let build = |lead_in: u64| {
        let mut b = item("b1", 1000, "one");
        {
            let n = b.narration.as_mut().unwrap();
            n.lead_in_ms = DurationMs::millis(lead_in);
            n.tail_ms = DurationMs::millis(300 - lead_in);
        }
        timeline(vec![b])
    };
    let before = build(150);
    let after = build(50);
    assert_eq!(
        before.duration_ms, after.duration_ms,
        "the fixture must keep the total fixed, or it proves nothing"
    );

    let d = diff(&before, &after);
    assert!(!d.is_empty());
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].reason, ChangeReason::PaddingChanged);
}

/// The estimated-to-measured transition is real drift — the video did change
/// — but it is not an author's edit, and conflating the two would send a
/// reader to re-read prose that nobody touched.
#[test]
fn an_estimate_becoming_a_measurement_is_named_as_such() {
    let before = {
        let mut b = item("b1", 4000, "one");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Estimated;
        timeline(vec![b])
    };
    let after = {
        let mut b = item("b1", 4300, "one");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Measured;
        timeline(vec![b])
    };

    let d = diff(&before, &after);
    assert!(!d.is_empty());
    let changed = d
        .changed
        .iter()
        .find(|c| c.item == "b1")
        .expect("b1 changed");
    assert_eq!(changed.reason, ChangeReason::NowMeasured);

    let rendered = d.render();
    assert!(rendered.contains("now measured"), "{rendered}");
}

/// The case above only reported because the duration also moved. With the
/// `null` backend it never moves: `NullVoice::synthesize` renders exactly
/// what `WpmEstimator` predicts, so every real estimate-to-measurement
/// transition has an identical length on both sides. If the reason arm sits
/// behind a guard that ignores `duration_source`, this is the case that goes
/// silently clean — the same defect the review named for `manifest_diff`.
#[test]
fn a_measurement_that_matches_its_estimate_still_reports() {
    let before = {
        let mut b = item("b1", 4000, "one");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Estimated;
        timeline(vec![b])
    };
    let after = {
        let mut b = item("b1", 4000, "one");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Measured;
        timeline(vec![b])
    };
    assert_eq!(
        before.duration_ms, after.duration_ms,
        "the fixture must hold the length fixed, or it proves nothing"
    );

    let d = diff(&before, &after);
    assert!(
        !d.is_empty(),
        "an unchanged length must not hide the measurement"
    );
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].reason, ChangeReason::NowMeasured);
}

/// The reverse — a cleared cache turning a measurement back into an estimate
/// — is not a change to the program, and must stay clean. This is why the
/// guard tests the transition rather than a bare `duration_source`
/// inequality: a symmetric guard would report this as "padding changed".
#[test]
fn a_cleared_cache_is_not_drift() {
    let before = {
        let mut b = item("b1", 4000, "one");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Measured;
        timeline(vec![b])
    };
    let after = {
        let mut b = item("b1", 4000, "one");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Estimated;
        timeline(vec![b])
    };

    let d = diff(&before, &after);
    assert!(d.is_empty(), "{:?}", d.changed);
}

/// A text edit that also happens to cross the estimated/measured boundary is
/// still a text edit — that is the cause the author can act on.
#[test]
fn a_text_edit_outranks_the_measurement_transition() {
    let before = {
        let mut b = item("b1", 4000, "one");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Estimated;
        timeline(vec![b])
    };
    let after = {
        let mut b = item("b1", 4300, "two");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Measured;
        timeline(vec![b])
    };

    let d = diff(&before, &after);
    assert_eq!(d.changed[0].reason, ChangeReason::TextEdited);
}

/// `diff --format json` publishes each reason as its prose; the enum must
/// not change that.
#[test]
fn a_change_reason_serializes_as_its_prose() {
    let cases = [
        (ChangeReason::TextEdited, "\"text edited\""),
        (ChangeReason::NowMeasured, "\"now measured\""),
        (ChangeReason::AudioChanged, "\"audio changed\""),
        (ChangeReason::PaddingChanged, "\"padding changed\""),
    ];
    for (reason, json) in cases {
        assert_eq!(serde_json::to_string(&reason).unwrap(), json);
    }
}
