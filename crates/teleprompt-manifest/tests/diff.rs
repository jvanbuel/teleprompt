use teleprompt_core::Hash;
use teleprompt_core::{DurationSource, SpanMs, TimeMs};
use teleprompt_manifest::diff::{diff, DriftReason};
use teleprompt_manifest::{
    AudioInfo, ChapterEntry, LineEntry, NarrationManifest, MANIFEST_VERSION,
};

fn seg(id: &str, start_ms: u64, duration_ms: u64, text: &str, audio_seed: &str) -> LineEntry {
    LineEntry {
        speaker: None,
        id: id.into(),
        text: text.to_string(),
        chapter: "intro".to_string(),
        start_ms: TimeMs::at(start_ms),
        duration_ms: SpanMs::of(duration_ms),
        duration_source: DurationSource::Measured,
        audio: format!("audio/{id}.wav"),
        source_hash: Hash::of(text.as_bytes()),
        audio_hash: Hash::of(audio_seed.as_bytes()),
        words: None,
        tempo_permille: None,
    }
}

fn manifest(lines: Vec<LineEntry>) -> NarrationManifest {
    let duration_ms = lines.last().map_or(SpanMs::ZERO, |s| {
        (s.start_ms + s.duration_ms) - TimeMs::ZERO
    });
    NarrationManifest {
        manifest_version: MANIFEST_VERSION,
        script: "tour.md".to_string(),
        locale: "en".to_string(),
        generated_by: "teleprompt 0.1.0".to_string(),
        duration_ms,
        audio: AudioInfo {
            format: "wav".into(),
            sample_rate: 48_000,
            channels: 1,
        },
        chapters: Vec::new(),
        lines,
        shots: Vec::new(),
    }
}

#[test]
fn an_unchanged_manifest_reports_nothing() {
    let m = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let d = diff(&m, &m);
    assert!(d.is_empty());
}

#[test]
fn an_edited_paragraph_is_reported_as_a_text_edit() {
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![seg(
        "welcome",
        0,
        2000,
        "Hello there, at length.",
        "a",
    )]);
    let d = diff(&before, &after);

    assert!(!d.is_empty());
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].id, "welcome");
    assert_eq!(d.changed[0].before_ms, 1000);
    assert_eq!(d.changed[0].after_ms, 2000);
    assert_eq!(d.changed[0].reason, DriftReason::TextEdited);
}

#[test]
fn a_new_voice_is_reported_as_an_audio_change_not_a_text_edit() {
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![seg("welcome", 0, 1100, "Hello.", "b")]);
    let d = diff(&before, &after);

    assert_eq!(d.changed[0].reason, DriftReason::AudioChanged);
}

#[test]
fn a_line_that_only_shifted_is_still_drift() {
    // Position-only change: everything else about the line (text, audio,
    // voice) is identical. Its audio is byte-identical, so this is drift a
    // consumer must still resynchronise against, but it is not "audio
    // changed" — nothing about the audio changed, only where it lands.
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![seg("welcome", 500, 1000, "Hello.", "a")]);
    let d = diff(&before, &after);

    assert!(
        !d.is_empty(),
        "a shifted line desynchronises every consumer"
    );
    assert_eq!(d.changed[0].reason, DriftReason::Shifted);
}

#[test]
fn a_duration_only_change_is_audio_changed_not_shifted() {
    // Same start_ms, only duration_ms differs (audio_hash unchanged — e.g.
    // a re-synthesis that landed on the same bytes-hash bucket but a
    // different measured length). Nothing moved, so this must not be
    // reported as "shifted".
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![seg("welcome", 0, 1200, "Hello.", "a")]);
    let d = diff(&before, &after);

    assert_eq!(d.changed[0].reason, DriftReason::AudioChanged);
}

#[test]
fn a_chapter_title_edit_is_drift_even_with_identical_lines() {
    // "Quick start" -> "Quick Start" slugifies to the same chapter id, and
    // touches no LineEntry field at all — this is the case is_empty()
    // used to miss entirely.
    let lines = vec![seg("welcome", 0, 1000, "Hello.", "a")];
    let mut before = manifest(lines.clone());
    before.chapters = vec![ChapterEntry {
        id: "quick-start".to_string(),
        title: "Quick start".to_string(),
        start_ms: TimeMs::at(0),
    }];
    let mut after = manifest(lines);
    after.chapters = vec![ChapterEntry {
        id: "quick-start".to_string(),
        title: "Quick Start".to_string(),
        start_ms: TimeMs::at(0),
    }];

    let d = diff(&before, &after);
    assert!(d.chapters_changed);
    assert!(!d.is_empty(), "a chapter title edit is drift");
}

#[test]
fn a_changed_sample_rate_is_drift() {
    let lines = vec![seg("welcome", 0, 1000, "Hello.", "a")];
    let mut before = manifest(lines.clone());
    before.audio.sample_rate = 48_000;
    let mut after = manifest(lines);
    after.audio.sample_rate = 44_100;

    let d = diff(&before, &after);
    assert!(d.audio_changed);
    assert!(!d.is_empty(), "a changed sample rate is drift");
}

#[test]
fn editing_the_first_segment_shifts_later_lines_without_flagging_them_for_rerender() {
    let one = seg("one", 0, 500, "Hi.", "a");
    let two = seg("two", 500, 500, "Two.", "b");
    let three = seg("three", 1000, 500, "Three.", "c");
    let before = manifest(vec![one, two.clone(), three.clone()]);

    // Editing "one"'s text grows its duration from 500ms to 800ms, which
    // pushes "two" and "three" 300ms later. Their own text, audio, and
    // duration are all untouched.
    let one_after = seg("one", 0, 800, "Hi there, at length now.", "a");
    let mut two_after = two.clone();
    two_after.start_ms = TimeMs::at(800);
    let mut three_after = three.clone();
    three_after.start_ms = TimeMs::at(1300);
    let after = manifest(vec![one_after, two_after, three_after]);

    let d = diff(&before, &after);
    assert_eq!(d.changed.len(), 3);

    let one_c = d.changed.iter().find(|c| c.id == "one").unwrap();
    assert_eq!(one_c.reason, DriftReason::TextEdited);
    let two_c = d.changed.iter().find(|c| c.id == "two").unwrap();
    assert_eq!(two_c.reason, DriftReason::Shifted);
    let three_c = d.changed.iter().find(|c| c.id == "three").unwrap();
    assert_eq!(three_c.reason, DriftReason::Shifted);

    let text = d.render();
    let rerender_section = text
        .split("needs re-render:\n")
        .nth(1)
        .expect("a needs re-render section");
    assert!(rerender_section.contains("one"), "{text}");
    assert!(!rerender_section.contains("two"), "{text}");
    assert!(!rerender_section.contains("three"), "{text}");
}

#[test]
fn a_shorter_manifest_renders_a_minus_sign_on_the_delta() {
    let before = manifest(vec![seg(
        "welcome",
        0,
        2000,
        "Hello there, at length.",
        "a",
    )]);
    let after = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let text = diff(&before, &after).render();

    assert!(text.contains("(-1.0s)"), "{text}");
}

#[test]
fn added_and_removed_lines_are_named() {
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![
        seg("welcome", 0, 1000, "Hello.", "a"),
        seg("outro", 1000, 500, "Bye.", "c"),
    ]);
    let d = diff(&before, &after);
    assert_eq!(d.added, vec!["outro"]);
    assert!(d.removed.is_empty());

    let back = diff(&after, &before);
    assert_eq!(back.removed, vec!["outro"]);
    assert!(back.added.is_empty());
}

#[test]
fn reordering_two_unchanged_lines_is_drift() {
    let a = seg("one", 0, 1000, "First.", "a");
    let mut b = seg("two", 1000, 1000, "Second.", "b");
    let before = manifest(vec![a.clone(), b.clone()]);

    b.start_ms = TimeMs::at(0);
    let mut a2 = a.clone();
    a2.start_ms = TimeMs::at(1000);
    let after = manifest(vec![b, a2]);

    let d = diff(&before, &after);
    assert!(
        d.reordered,
        "order is part of the contract, not an accident"
    );
    assert!(!d.is_empty());
}

#[test]
fn the_report_names_the_duration_change_and_the_lines() {
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![seg(
        "welcome",
        0,
        2000,
        "Hello there, at length.",
        "a",
    )]);
    let text = diff(&before, &after).render();

    assert!(text.contains("1.0s"), "{text}");
    assert!(text.contains("2.0s"), "{text}");
    assert!(text.contains("welcome"), "{text}");
    assert!(text.contains("text edited"), "{text}");
    assert!(text.contains("needs re-render"), "{text}");
}

/// The manifest's `duration_source` used to be inert: `reason_for` never
/// looked at it, so a line that went from a prediction to a measurement
/// with the same length — which with `null` is every line, because the
/// estimator and the backend call the same function — was reported as clean.
/// A committed manifest full of estimates would have passed `--check`.
#[test]
fn an_estimated_line_that_becomes_measured_is_drift() {
    let mut before = seg("welcome", 0, 2750, "One two three four five six.", "a");
    before.duration_source = DurationSource::Estimated;
    let after = seg("welcome", 0, 2750, "One two three four five six.", "a");

    let d = diff(&manifest(vec![before]), &manifest(vec![after]));
    assert_eq!(d.changed.len(), 1, "{:?}", d.changed);
    assert_eq!(d.changed[0].reason, DriftReason::NowMeasured);
}

/// The reverse — a cleared cache turning a measurement back into a
/// prediction — is still drift, but it must not claim something was just
/// measured. Mirrors the timeline diff's own reading of the same
/// transition.
#[test]
fn a_measured_line_that_reverts_to_estimated_does_not_claim_a_measurement() {
    let before = seg("welcome", 0, 2750, "One two three four five six.", "a");
    let mut after = seg("welcome", 0, 2750, "One two three four five six.", "a");
    after.duration_source = DurationSource::Estimated;

    let d = diff(&manifest(vec![before]), &manifest(vec![after]));
    assert_ne!(
        d.changed.first().map(|c| c.reason),
        Some(DriftReason::NowMeasured)
    );
}

/// `now measured` sits after `text edited`, so an author who rewrote a
/// paragraph is told about their own edit rather than about the cache.
#[test]
fn a_text_edit_outranks_the_measurement_transition() {
    let mut before = seg("welcome", 0, 2750, "One two three four five six.", "a");
    before.duration_source = DurationSource::Estimated;
    let after = seg("welcome", 0, 3000, "One two three four five six more.", "b");

    let d = diff(&manifest(vec![before]), &manifest(vec![after]));
    assert_eq!(d.changed[0].reason, DriftReason::TextEdited);
}

/// And before `audio changed`, so the transition is named rather than being
/// absorbed into a report that sends the reader hunting for a content
/// change that did not happen.
#[test]
fn the_measurement_transition_outranks_a_length_change() {
    let mut before = seg("welcome", 0, 2750, "One two three four five six.", "a");
    before.duration_source = DurationSource::Estimated;
    let after = seg("welcome", 0, 5000, "One two three four five six.", "b");

    let d = diff(&manifest(vec![before]), &manifest(vec![after]));
    assert_eq!(d.changed[0].reason, DriftReason::NowMeasured);
}

/// `dub --check --format json` publishes each reason as its prose; the enum
/// must not change that.
#[test]
fn a_drift_reason_serializes_as_its_prose() {
    let cases = [
        (DriftReason::TextEdited, "\"text edited\""),
        (DriftReason::NowMeasured, "\"now measured\""),
        (DriftReason::AudioChanged, "\"audio changed\""),
        (DriftReason::Shifted, "\"shifted\""),
    ];
    for (reason, json) in cases {
        assert_eq!(serde_json::to_string(&reason).unwrap(), json);
    }
}
