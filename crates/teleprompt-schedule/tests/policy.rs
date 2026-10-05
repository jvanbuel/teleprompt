use teleprompt_core::config::TimingConfig;
use teleprompt_core::policy::{Align, PolicyKind};
use teleprompt_core::DurationMs;
use teleprompt_schedule::{layout, layout_at};

fn timing() -> TimingConfig {
    TimingConfig {
        lead_in_ms: DurationMs::millis(150),
        tail_ms: DurationMs::millis(150),
        max_stretch: 3.0,
        min_stretch: 0.33,
        trim_warn_above: 2.0,
        ..teleprompt_core::config::Config::default().timing
    }
}

#[test]
fn hold_runs_the_action_after_the_narration() {
    let l = layout(PolicyKind::Hold, Align::Start, 5000, 800, &timing());
    assert_eq!(l.narration_start_ms, 0);
    assert_eq!(l.action_start_ms, 5000);
    assert_eq!(l.action_duration_ms, 800);
    assert_eq!(l.item_duration_ms, 5800);
}

#[test]
fn hold_with_no_action_is_just_the_narration() {
    let l = layout(PolicyKind::Hold, Align::Start, 5000, 0, &timing());
    assert_eq!(l.item_duration_ms, 5000);
}

#[test]
fn concurrent_start_begins_both_together() {
    let l = layout(PolicyKind::Concurrent, Align::Start, 5000, 800, &timing());
    assert_eq!(l.narration_start_ms, 0);
    assert_eq!(l.action_start_ms, 0);
    assert_eq!(l.item_duration_ms, 5000, "the longer of the two");
}

#[test]
fn concurrent_start_uses_the_action_when_it_is_longer() {
    let l = layout(PolicyKind::Concurrent, Align::Start, 800, 5000, &timing());
    assert_eq!(l.item_duration_ms, 5000);
}

#[test]
fn concurrent_end_finishes_both_together() {
    let l = layout(PolicyKind::Concurrent, Align::End, 5000, 800, &timing());
    assert_eq!(l.narration_start_ms, 0);
    assert_eq!(l.action_start_ms, 4200);
    assert_eq!(l.action_start_ms + l.action_duration_ms, l.item_duration_ms);
}

#[test]
fn concurrent_center_centres_the_shorter_one() {
    let l = layout(PolicyKind::Concurrent, Align::Center, 5000, 800, &timing());
    assert_eq!(l.action_start_ms, 2100, "(5000 - 800) / 2");
    assert_eq!(l.narration_start_ms, 0);
}

#[test]
fn stretch_makes_the_action_exactly_fill_the_narration() {
    let l = layout(PolicyKind::FitAction, Align::Start, 5000, 2500, &timing());
    assert_eq!(l.action_duration_ms, 5000);
    assert_eq!(l.item_duration_ms, 5000);
    assert!(l.warnings.is_empty());
}

#[test]
fn stretch_compresses_a_long_action_too() {
    let l = layout(PolicyKind::FitAction, Align::Start, 2000, 4000, &timing());
    assert_eq!(l.action_duration_ms, 2000);
}

#[test]
fn stretch_beyond_the_maximum_is_clamped_and_warns() {
    // 500ms action asked to fill 5000ms is a factor of 10, above max_stretch 3.0
    let l = layout(PolicyKind::FitAction, Align::Start, 5000, 500, &timing());
    assert_eq!(l.action_duration_ms, 1500, "500 * 3.0");
    assert_eq!(l.item_duration_ms, 5000, "narration still governs");
    assert!(l.warnings[0].contains("max_stretch"));
    assert!(
        l.warnings[0].contains("action"),
        "the warning must name what is being stretched, or it reads as if the \
         speech were resampled: {}",
        l.warnings[0]
    );
}

#[test]
fn stretch_below_the_minimum_is_clamped_and_warns() {
    // 10000ms action asked to fit 1000ms is a factor of 0.1, below min_stretch 0.33
    let l = layout(PolicyKind::FitAction, Align::Start, 1000, 10_000, &timing());
    assert_eq!(l.action_duration_ms, 3300, "10000 * 0.33");
    assert_eq!(l.item_duration_ms, 3300, "the clamped action now governs");
    assert!(l.warnings[0].contains("min_stretch"));
    assert!(
        l.warnings[0].contains("action"),
        "the warning must name what is being stretched, or it reads as if the \
         speech were resampled: {}",
        l.warnings[0]
    );
}

#[test]
fn stretch_with_a_zero_length_action_does_not_divide_by_zero() {
    let l = layout(PolicyKind::FitAction, Align::Start, 5000, 0, &timing());
    assert_eq!(l.action_duration_ms, 0);
    assert_eq!(l.item_duration_ms, 5000);
}

#[test]
fn trim_leaves_a_short_action_alone() {
    let l = layout(PolicyKind::TrimAction, Align::Start, 5000, 800, &timing());
    assert_eq!(l.action_duration_ms, 800);
    assert_eq!(l.item_duration_ms, 5000, "narration is authoritative");
}

#[test]
fn trim_speeds_up_an_over_long_action_within_the_bound() {
    let l = layout(PolicyKind::TrimAction, Align::Start, 5000, 8000, &timing());
    assert_eq!(l.action_duration_ms, 5000, "8000 / 1.6 fits exactly");
    assert!(l.warnings.is_empty());
}

#[test]
fn trim_beyond_trim_warn_above_cuts_and_warns() {
    // 20000ms into 5000ms needs 4x, above trim_warn_above 2.0
    let l = layout(
        PolicyKind::TrimAction,
        Align::Start,
        5000,
        20_000,
        &timing(),
    );
    assert_eq!(l.action_duration_ms, 5000, "cut to the narration length");
    assert_eq!(l.item_duration_ms, 5000);
    assert!(l.warnings[0].contains("trim_warn_above"));
}

/// The divide-by-zero this guards against is real; zeroing the action to
/// avoid it was not. An action with no narration to trim against keeps its
/// own length, which is what the tape says and the only number available.
#[test]
fn trim_with_zero_narration_does_not_divide_by_zero() {
    let l = layout(PolicyKind::TrimAction, Align::Start, 0, 5000, &timing());
    assert_eq!(l.item_duration_ms, 5000);
    assert_eq!(l.action_duration_ms, 5000);
}

/// The property the policy names fight: no policy modifies the narration.
///
/// `Layout` has no narration-duration field at all — narration is an input to
/// `layout` and never an output — so the only way a policy could shorten
/// speech is by leaving it no room in the item. Every policy, at every ratio,
/// must give the narration its full length inside the item. `stretch` and
/// `trim` adjust the *action*; the audio is played as the voice produced it.
#[test]
fn no_policy_ever_denies_the_narration_its_full_length() {
    let policies = [
        (PolicyKind::Hold, Align::Start),
        (PolicyKind::Concurrent, Align::Start),
        (PolicyKind::Concurrent, Align::End),
        (PolicyKind::Concurrent, Align::Center),
        (PolicyKind::FitAction, Align::Start),
        (PolicyKind::TrimAction, Align::Start),
        (PolicyKind::FitLine, Align::Start),
    ];
    // Ratios spanning both clamp directions: action far shorter than
    // narration, comparable, and far longer.
    let cases = [
        (5000u64, 0u64),
        (5000, 500),
        (5000, 5000),
        (5000, 20_000),
        (1000, 10_000),
    ];

    for (policy, align) in policies {
        for (narration_ms, action_ms) in cases {
            let l = layout(policy, align, narration_ms, action_ms, &timing());
            assert!(
                l.narration_start_ms + narration_ms <= l.item_duration_ms,
                "{} with narration {}ms / action {}ms starts narration at {}ms \
                 in a {}ms item, cutting {}ms of speech",
                policy.label(),
                narration_ms,
                action_ms,
                l.narration_start_ms,
                l.item_duration_ms,
                (l.narration_start_ms + narration_ms).saturating_sub(l.item_duration_ms),
            );
        }
    }
}

/// A shot after a mark has no narration of its own: the paragraph belongs
/// to the block's first shot and the rest run under what the policy left of
/// it. There is nothing for such an action to fill, so it keeps its own
/// length — clamping it to `min_stretch` cut it to a third of what the tape
/// says, which is the scheduler rewriting a tape it was never asked about.
#[test]
fn stretching_against_no_narration_leaves_the_action_alone() {
    let l = layout(PolicyKind::FitAction, Align::Start, 0, 4000, &timing());
    assert_eq!(l.action_duration_ms, 4000);
    assert_eq!(l.item_duration_ms, 4000);
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
}

/// The same in the other direction: `trim-action` with nothing to trim
/// against cut the action to zero, which deletes it from the video.
#[test]
fn trimming_against_no_narration_leaves_the_action_alone() {
    let l = layout(PolicyKind::TrimAction, Align::Start, 0, 4000, &timing());
    assert_eq!(l.action_duration_ms, 4000);
    assert_eq!(l.item_duration_ms, 4000);
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
}

/// A shot anchors the action to a moment inside the narration: the terminal
/// should be typing the command at the moment the voice names it, not at
/// the top of the paragraph and not after it.
#[test]
fn a_cue_starts_the_action_where_the_words_are() {
    let l = layout_at(
        PolicyKind::Concurrent,
        Align::Start,
        10_000,
        3_000,
        Some(4_000),
        &timing(),
    );
    assert_eq!(l.narration_start_ms, 0);
    assert_eq!(l.action_start_ms, 4_000, "the action waits for its shot");
    assert_eq!(l.action_duration_ms, 3_000);
    assert_eq!(
        l.item_duration_ms, 10_000,
        "and still fits inside the paragraph"
    );
}

/// An action cued late enough to outlast the sentence extends the item
/// rather than being cut off by it.
#[test]
fn a_cue_near_the_end_lengthens_the_shot_rather_than_clipping_the_action() {
    let l = layout_at(
        PolicyKind::Concurrent,
        Align::Start,
        10_000,
        4_000,
        Some(8_000),
        &timing(),
    );
    assert_eq!(l.action_start_ms, 8_000);
    assert_eq!(l.item_duration_ms, 12_000);
}

/// A recorded take is stretched less than a synthesized line: a real
/// voice sounds stretched sooner.
#[test]
fn a_take_keeps_to_tighter_bounds_than_a_synthesized_line() {
    use teleprompt_schedule::policy::fit_line;
    let line = fit_line(2000, 10, 1000, false, &timing()).unwrap();
    let take = fit_line(2000, 10, 1000, true, &timing()).unwrap();
    assert_eq!(line.tempo_permille, 1150);
    assert_eq!(take.tempo_permille, 1080);
    assert!(take.warning.unwrap().contains("max_take_speed"));
}
