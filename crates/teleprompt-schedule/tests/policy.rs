use teleprompt_core::config::TimingConfig;
use teleprompt_schedule::{layout, Align, Policy};

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
}

#[test]
fn stretch_below_the_minimum_is_clamped_and_warns() {
    // 10000ms action asked to fit 1000ms is a factor of 0.1, below min_stretch 0.33
    let l = layout(Policy::Stretch, 1000, 10_000, &timing());
    assert_eq!(l.action_duration_ms, 3300, "10000 * 0.33");
    assert_eq!(l.beat_duration_ms, 3300, "the clamped action now governs");
    assert!(l.warnings[0].contains("min_stretch"));
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

#[test]
fn trim_with_zero_narration_does_not_divide_by_zero() {
    let l = layout(Policy::Trim, 0, 5000, &timing());
    assert_eq!(l.beat_duration_ms, 0);
    assert_eq!(l.action_duration_ms, 0);
}

#[test]
fn policy_parses_from_attribute_strings() {
    assert_eq!(Policy::parse("hold", "start"), Some(Policy::Hold));
    assert_eq!(
        Policy::parse("concurrent", "end"),
        Some(Policy::Concurrent(Align::End))
    );
    assert_eq!(Policy::parse("stretch", "start"), Some(Policy::Stretch));
    assert_eq!(Policy::parse("trim", "start"), Some(Policy::Trim));
    assert_eq!(Policy::parse("nonsense", "start"), None);
    assert_eq!(Policy::parse("concurrent", "sideways"), None);
}
