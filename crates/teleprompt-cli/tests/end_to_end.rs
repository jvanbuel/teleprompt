//! End-to-end acceptance for the feedback loop.
//!
//! These tests drive the CLI's own command functions against a realistic,
//! multi-chapter script (`tests/fixtures/tour.md`) rather than the small
//! synthetic fixtures used elsewhere. Together they answer the project's
//! defining question: does editing prose produce a legible, useful diff of the
//! video's pacing?

use std::path::PathBuf;
use teleprompt_core::SpanMs;
use teleprompt_core::TimeMs;

use teleprompt_project::project::Project;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("fixture must exist")
}

fn workspace() -> (teleprompt_testkit::TestDir, Project, PathBuf) {
    let dir = teleprompt_testkit::test_dir("e2e");
    teleprompt_project::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/tour.md");
    std::fs::write(&script, fixture("tour.md")).unwrap();
    let project = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    (dir, project, script)
}

#[test]
fn the_full_fixture_validates() {
    let (_dir, p, s) = workspace();
    let warnings = p.script(&s, "en").check().expect("fixture must be valid");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
}

#[test]
fn every_policy_appears_in_the_compiled_timeline() {
    let (_dir, p, s) = workspace();
    let out = p.script(&s, "en").plan().unwrap();
    let policies: std::collections::BTreeSet<&str> = out
        .timeline
        .entries
        .iter()
        .map(|e| e.policy.label())
        .collect();
    for expected in ["hold", "concurrent", "fit-action", "trim-action"] {
        assert!(policies.contains(expected), "missing policy {expected}");
    }
}

#[test]
fn items_never_overlap_and_never_gap() {
    let (_dir, p, s) = workspace();
    let out = p.script(&s, "en").plan().unwrap();
    for pair in out.timeline.entries.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let expected = a.start_ms + a.duration_ms - a.transition.duration_ms;
        assert_eq!(
            b.start_ms, expected,
            "gap or overlap between {} and {}",
            a.item, b.item
        );
    }
}

#[test]
fn the_timeline_ends_where_the_last_shot_ends() {
    let (_dir, p, s) = workspace();
    let out = p.script(&s, "en").plan().unwrap();
    let last = out.timeline.entries.last().unwrap();
    assert_eq!(
        TimeMs::ZERO + out.timeline.duration_ms,
        last.start_ms + last.duration_ms
    );
}

#[test]
fn planning_twice_gives_byte_identical_output() {
    let (_dir, p, s) = workspace();
    let a = serde_json::to_string(&p.script(&s, "en").plan().unwrap().timeline).unwrap();
    let b = serde_json::to_string(&p.script(&s, "en").plan().unwrap().timeline).unwrap();
    assert_eq!(a, b);
}

/// The acceptance criterion.
#[test]
fn editing_one_paragraph_shows_up_as_a_legible_pacing_diff() {
    let (_dir, p, s) = workspace();

    let baseline = p.script(&s, "en").plan().unwrap();
    let dest = p.timeline_path("tour.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(
        &dest,
        serde_json::to_string_pretty(&baseline.timeline).unwrap(),
    )
    .unwrap();

    let edited = fixture("tour.md").replace(
        "Deployment is one command, and it streams progress as it goes.",
        "Deployment is one single command, and it streams its progress as it goes along, \
         step by step, so you always know exactly where you are.",
    );
    std::fs::write(&s, edited).unwrap();

    let d = p.script(&s, "en").plan_check().unwrap();

    assert_eq!(d.changed.len(), 1, "exactly one line changed");
    assert_eq!(d.changed[0].item, "deploy");
    assert_eq!(
        d.changed[0].reason,
        teleprompt_schedule::ChangeReason::TextEdited
    );
    assert!(d.shift_ms > 0, "a longer paragraph lengthens the video");
    assert!(
        !d.recapture.is_empty(),
        "the stretched item needs re-capture"
    );

    let rendered = d.render();
    assert!(rendered.contains("deploy"));
    assert!(rendered.contains('→'));
}

/// Exact-duration assertion. Guards against silent regressions in text
/// fidelity: `rollback`'s soft-wrapped paragraph had its two lines concatenated
/// with no separator ("one" and "extra" fused into "oneextra"), which
/// undercounted its words by one and made every duration derived from wrapped
/// prose too short. Hand-derived from the null backend's documented model (see
/// `teleprompt_voice::null::estimate_ms`) rather than copied from program
/// output.
///
/// `rollback`'s item uses `policy=trim-action`, whose layout sets the item's
/// `duration_ms` to exactly the (padded) narration length regardless of the
/// action's duration (`teleprompt_schedule::policy::layout`, `Policy::Trim`
/// arm) — so this is also an exact assertion on the item, not just the
/// narration slot.
///
/// Line text (after soft-break-as-space joins the two source lines and
/// the `{#rollback}` attribute suffix is stripped):
///   "If a deploy goes wrong, rolling back takes the same single command
///    with one extra flag."
///
/// Word count (16, every token has an alphanumeric char):
///   If a deploy goes wrong, rolling back takes the same single command
///   with one extra flag.  -> 16 words
///
/// Speech time at 150 wpm: 16 / 150 * 60_000 = 6_400 ms
/// Punctuation pauses: 1 comma (150) + 1 sentence-ending period (350) = 500 ms
/// Speed is 1.0 (default, unset by the fixture), so no division.
/// Narration duration_ms = 6_400 + 500 = 6_900 ms
/// Padded with 150 ms lead-in + 150 ms tail = 7_200 ms
#[test]
fn rollback_line_has_the_hand_derived_exact_duration() {
    let (_dir, p, s) = workspace();
    let out = p.script(&s, "en").plan().unwrap();
    let entry = out
        .timeline
        .entry("rollback")
        .expect("tour fixture has a `rollback` item");
    let narration = entry.narration.as_ref().expect("rollback item narrates");
    assert_eq!(
        narration.duration_ms,
        SpanMs::of(6_900),
        "unpadded narration length"
    );
    assert_eq!(
        entry.duration_ms,
        SpanMs::of(7_200),
        "trim policy sets item duration_ms to the padded narration length"
    );
}

#[test]
fn an_unedited_script_diffs_clean_against_its_committed_timeline() {
    let (_dir, p, s) = workspace();
    let out = p.script(&s, "en").plan().unwrap();
    let dest = p.timeline_path("tour.md", "en");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, serde_json::to_string_pretty(&out.timeline).unwrap()).unwrap();

    let d = p.script(&s, "en").plan_check().unwrap();
    assert!(
        d.is_empty(),
        "clean checkout must diff clean: {}",
        d.render()
    );
}
