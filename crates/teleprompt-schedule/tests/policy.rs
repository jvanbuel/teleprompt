use teleprompt_core::config::TimingConfig;
use teleprompt_schedule::{layout, layout_at, Align, Policy};

fn timing() -> TimingConfig {
    TimingConfig {
        lead_in_ms: 150,
        tail_ms: 150,
        max_stretch: 3.0,
        min_stretch: 0.33,
        max_speedup: 2.0,
    }
}

#[test]
fn hold_runs_the_action_after_the_narration() {
    let l = layout(Policy::Hold, 5000, 800, &timing());
    assert_eq!(l.narration_start_ms, 0);
    assert_eq!(l.action_start_ms, 5000);
    assert_eq!(l.action_duration_ms, 800);
    assert_eq!(l.beat_duration_ms, 5800);
}

#[test]
fn hold_with_no_action_is_just_the_narration() {
    let l = layout(Policy::Hold, 5000, 0, &timing());
    assert_eq!(l.beat_duration_ms, 5000);
}

#[test]
fn concurrent_start_begins_both_together() {
    let l = layout(Policy::Concurrent(Align::Start), 5000, 800, &timing());
    assert_eq!(l.narration_start_ms, 0);
    assert_eq!(l.action_start_ms, 0);
    assert_eq!(l.beat_duration_ms, 5000, "the longer of the two");
}

#[test]
fn concurrent_start_uses_the_action_when_it_is_longer() {
    let l = layout(Policy::Concurrent(Align::Start), 800, 5000, &timing());
    assert_eq!(l.beat_duration_ms, 5000);
}

#[test]
fn concurrent_end_finishes_both_together() {
    let l = layout(Policy::Concurrent(Align::End), 5000, 800, &timing());
    assert_eq!(l.narration_start_ms, 0);
    assert_eq!(l.action_start_ms, 4200);
    assert_eq!(l.action_start_ms + l.action_duration_ms, l.beat_duration_ms);
}

#[test]
fn concurrent_center_centres_the_shorter_one() {
    let l = layout(Policy::Concurrent(Align::Center), 5000, 800, &timing());
    assert_eq!(l.action_start_ms, 2100, "(5000 - 800) / 2");
    assert_eq!(l.narration_start_ms, 0);
}

#[test]
fn stretch_makes_the_action_exactly_fill_the_narration() {
    let l = layout(Policy::Stretch, 5000, 2500, &timing());
    assert_eq!(l.action_duration_ms, 5000);
    assert_eq!(l.beat_duration_ms, 5000);
    assert!(l.warnings.is_empty());
}

#[test]
fn stretch_compresses_a_long_action_too() {
    let l = layout(Policy::Stretch, 2000, 4000, &timing());
    assert_eq!(l.action_duration_ms, 2000);
}

#[test]
fn stretch_beyond_the_maximum_is_clamped_and_warns() {
    // 500ms action asked to fill 5000ms is a factor of 10, above max_stretch 3.0
    let l = layout(Policy::Stretch, 5000, 500, &timing());
    assert_eq!(l.action_duration_ms, 1500, "500 * 3.0");
    assert_eq!(l.beat_duration_ms, 5000, "narration still governs");
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
    let l = layout(Policy::Stretch, 1000, 10_000, &timing());
    assert_eq!(l.action_duration_ms, 3300, "10000 * 0.33");
    assert_eq!(l.beat_duration_ms, 3300, "the clamped action now governs");
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
    let l = layout(Policy::Stretch, 5000, 0, &timing());
    assert_eq!(l.action_duration_ms, 0);
    assert_eq!(l.beat_duration_ms, 5000);
}

#[test]
fn trim_leaves_a_short_action_alone() {
    let l = layout(Policy::Trim, 5000, 800, &timing());
    assert_eq!(l.action_duration_ms, 800);
    assert_eq!(l.beat_duration_ms, 5000, "narration is authoritative");
}

#[test]
fn trim_speeds_up_an_over_long_action_within_the_bound() {
    let l = layout(Policy::Trim, 5000, 8000, &timing());
    assert_eq!(l.action_duration_ms, 5000, "8000 / 1.6 fits exactly");
    assert!(l.warnings.is_empty());
}

#[test]
fn trim_beyond_max_speedup_cuts_and_warns() {
    // 20000ms into 5000ms needs 4x, above max_speedup 2.0
    let l = layout(Policy::Trim, 5000, 20_000, &timing());
    assert_eq!(l.action_duration_ms, 5000, "cut to the narration length");
    assert_eq!(l.beat_duration_ms, 5000);
    assert!(l.warnings[0].contains("max_speedup"));
}

/// The divide-by-zero this guards against is real; zeroing the action to
/// avoid it was not. An action with no narration to trim against keeps its
/// own length, which is what the tape says and the only number available.
#[test]
fn trim_with_zero_narration_does_not_divide_by_zero() {
    let l = layout(Policy::Trim, 0, 5000, &timing());
    assert_eq!(l.beat_duration_ms, 5000);
    assert_eq!(l.action_duration_ms, 5000);
}

#[test]
fn policy_parses_from_attribute_strings() {
    assert_eq!(Policy::parse("hold", "start"), Some(Policy::Hold));
    assert_eq!(
        Policy::parse("concurrent", "end"),
        Some(Policy::Concurrent(Align::End))
    );
    assert_eq!(
        Policy::parse("stretch-action", "start"),
        Some(Policy::Stretch)
    );
    assert_eq!(Policy::parse("trim-action", "start"), Some(Policy::Trim));
    assert_eq!(Policy::parse("nonsense", "start"), None);
    assert_eq!(Policy::parse("concurrent", "sideways"), None);
}

/// The property the policy names fight: no policy modifies the narration.
///
/// `Layout` has no narration-duration field at all — narration is an input to
/// `layout` and never an output — so the only way a policy could shorten
/// speech is by leaving it no room in the beat. Every policy, at every ratio,
/// must give the narration its full length inside the beat. `stretch` and
/// `trim` adjust the *action*; the audio is played as the voice produced it.
#[test]
fn no_policy_ever_denies_the_narration_its_full_length() {
    let policies = [
        Policy::Hold,
        Policy::Concurrent(Align::Start),
        Policy::Concurrent(Align::End),
        Policy::Concurrent(Align::Center),
        Policy::Stretch,
        Policy::Trim,
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

    for policy in policies {
        for (narration_ms, action_ms) in cases {
            let l = layout(policy, narration_ms, action_ms, &timing());
            assert!(
                l.narration_start_ms + narration_ms <= l.beat_duration_ms,
                "{} with narration {}ms / action {}ms starts narration at {}ms \
                 in a {}ms beat, cutting {}ms of speech",
                policy.label(),
                narration_ms,
                action_ms,
                l.narration_start_ms,
                l.beat_duration_ms,
                (l.narration_start_ms + narration_ms).saturating_sub(l.beat_duration_ms),
            );
        }
    }
}

/// Issue #1: `stretch` and `trim` named an operation without naming its
/// object, so both read as if the speech were adjusted. The old spellings are
/// gone, not aliased — but `check` must say what to write instead rather than
/// reporting a bare "unknown policy".
#[test]
fn the_old_policy_names_are_rejected_with_the_new_spelling() {
    assert_eq!(Policy::parse("stretch", "start"), None);
    assert_eq!(Policy::parse("trim", "start"), None);

    assert_eq!(Policy::renamed_hint("stretch"), Some("stretch-action"));
    assert_eq!(Policy::renamed_hint("trim"), Some("trim-action"));
    assert_eq!(Policy::renamed_hint("hold"), None);
    assert_eq!(Policy::renamed_hint("nonsense"), None);
}

#[test]
fn labels_name_what_the_policy_adjusts() {
    assert_eq!(Policy::Stretch.label(), "stretch-action");
    assert_eq!(Policy::Trim.label(), "trim-action");
    assert_eq!(Policy::Hold.label(), "hold");
    assert_eq!(Policy::Concurrent(Align::Start).label(), "concurrent");
}

/// A span after a mark has no narration of its own: the paragraph belongs
/// to the block's first span and the rest run under what the policy left of
/// it. There is nothing for such an action to fill, so it keeps its own
/// length — clamping it to `min_stretch` cut it to a third of what the tape
/// says, which is the scheduler rewriting a tape it was never asked about.
#[test]
fn stretching_against_no_narration_leaves_the_action_alone() {
    let l = layout(Policy::Stretch, 0, 4000, &timing());
    assert_eq!(l.action_duration_ms, 4000);
    assert_eq!(l.beat_duration_ms, 4000);
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
}

/// The same in the other direction: `trim-action` with nothing to trim
/// against cut the action to zero, which deletes it from the video.
#[test]
fn trimming_against_no_narration_leaves_the_action_alone() {
    let l = layout(Policy::Trim, 0, 4000, &timing());
    assert_eq!(l.action_duration_ms, 4000);
    assert_eq!(l.beat_duration_ms, 4000);
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
}

/// A cue anchors the action to a moment inside the narration: the terminal
/// should be typing the command at the moment the voice names it, not at
/// the top of the paragraph and not after it.
#[test]
fn a_cue_starts_the_action_where_the_words_are() {
    let l = layout_at(
        Policy::Concurrent(Align::Start),
        10_000,
        3_000,
        Some(4_000),
        &timing(),
    );
    assert_eq!(l.narration_start_ms, 0);
    assert_eq!(l.action_start_ms, 4_000, "the action waits for its cue");
    assert_eq!(l.action_duration_ms, 3_000);
    assert_eq!(
        l.beat_duration_ms, 10_000,
        "and still fits inside the paragraph"
    );
}

/// An action cued late enough to outlast the sentence extends the beat
/// rather than being cut off by it.
#[test]
fn a_cue_near_the_end_lengthens_the_beat_rather_than_clipping_the_action() {
    let l = layout_at(
        Policy::Concurrent(Align::Start),
        10_000,
        4_000,
        Some(8_000),
        &timing(),
    );
    assert_eq!(l.action_start_ms, 8_000);
    assert_eq!(l.beat_duration_ms, 12_000);
}
