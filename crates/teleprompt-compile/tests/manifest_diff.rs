use teleprompt_compile::manifest::{
    AudioInfo, ChapterEntry, LineEntry, NarrationManifest, MANIFEST_VERSION,
};
use teleprompt_compile::manifest_diff::diff;
use teleprompt_core::Hash;

fn seg(id: &str, start_ms: u64, duration_ms: u64, text: &str, audio_seed: &str) -> LineEntry {
    LineEntry {
        id: id.to_string(),
        text: text.to_string(),
        chapter: "intro".to_string(),
        start_ms,
        duration_ms,
        duration_source: "measured".to_string(),
        audio: format!("audio/{id}.wav"),
        voice_source: "synthetic".to_string(),
        voice_source_actual: "synthetic".to_string(),
        downgrade_reason: None,
        source_hash: Hash::of(text.as_bytes()),
        audio_hash: Hash::of(audio_seed.as_bytes()),
        words: None,
    }
}

fn manifest(lines: Vec<LineEntry>) -> NarrationManifest {
    let duration_ms = lines
        .last()
        .map(|s| s.start_ms + s.duration_ms)
        .unwrap_or(0);
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
    assert_eq!(d.changed[0].reason, "text edited");
}

#[test]
fn a_new_voice_is_reported_as_an_audio_change_not_a_text_edit() {
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![seg("welcome", 0, 1100, "Hello.", "b")]);
    let d = diff(&before, &after);

    assert_eq!(d.changed[0].reason, "audio changed");
}

#[test]
fn a_segment_that_only_shifted_is_still_drift() {
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
    assert_eq!(d.changed[0].reason, "shifted");
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

    assert_eq!(d.changed[0].reason, "audio changed");
}

#[test]
fn a_voice_tier_downgrade_is_named() {
    let mut before_seg = seg("welcome", 0, 1000, "Hello.", "a");
    before_seg.voice_source = "cloned".to_string();
    before_seg.voice_source_actual = "cloned".to_string();

    let mut after_seg = before_seg.clone();
    after_seg.voice_source_actual = "synthetic".to_string();
    after_seg.downgrade_reason = Some("no voice profile enrolled".to_string());

    let before = manifest(vec![before_seg]);
    let after = manifest(vec![after_seg]);
    let d = diff(&before, &after);

    assert_eq!(d.changed[0].reason, "voice tier cloned → synthetic");
}

#[test]
fn a_voice_request_change_with_no_tier_change_is_named_separately() {
    let mut before_seg = seg("welcome", 0, 1000, "Hello.", "a");
    before_seg.voice_source = "cloned".to_string();
    before_seg.voice_source_actual = "synthetic".to_string();
    before_seg.downgrade_reason = Some("no voice profile enrolled".to_string());

    let mut after_seg = before_seg.clone();
    after_seg.voice_source = "synthetic".to_string();
    after_seg.downgrade_reason = None;

    let before = manifest(vec![before_seg]);
    let after = manifest(vec![after_seg]);
    let d = diff(&before, &after);

    assert_eq!(d.changed[0].reason, "voice request changed");
}

#[test]
fn a_chapter_title_edit_is_drift_even_with_identical_segments() {
    // "Quick start" -> "Quick Start" slugifies to the same chapter id, and
    // touches no LineEntry field at all — this is the case is_empty()
    // used to miss entirely.
    let lines = vec![seg("welcome", 0, 1000, "Hello.", "a")];
    let mut before = manifest(lines.clone());
    before.chapters = vec![ChapterEntry {
        id: "quick-start".to_string(),
        title: "Quick start".to_string(),
        start_ms: 0,
    }];
    let mut after = manifest(lines);
    after.chapters = vec![ChapterEntry {
        id: "quick-start".to_string(),
        title: "Quick Start".to_string(),
        start_ms: 0,
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
fn editing_the_first_segment_shifts_later_segments_without_flagging_them_for_rerender() {
    let one = seg("one", 0, 500, "Hi.", "a");
    let two = seg("two", 500, 500, "Two.", "b");
    let three = seg("three", 1000, 500, "Three.", "c");
    let before = manifest(vec![one, two.clone(), three.clone()]);

    // Editing "one"'s text grows its duration from 500ms to 800ms, which
    // pushes "two" and "three" 300ms later. Their own text, audio, and
    // duration are all untouched.
    let one_after = seg("one", 0, 800, "Hi there, at length now.", "a");
    let mut two_after = two.clone();
    two_after.start_ms = 800;
    let mut three_after = three.clone();
    three_after.start_ms = 1300;
    let after = manifest(vec![one_after, two_after, three_after]);

    let d = diff(&before, &after);
    assert_eq!(d.changed.len(), 3);

    let one_c = d.changed.iter().find(|c| c.id == "one").unwrap();
    assert_eq!(one_c.reason, "text edited");
    let two_c = d.changed.iter().find(|c| c.id == "two").unwrap();
    assert_eq!(two_c.reason, "shifted");
    let three_c = d.changed.iter().find(|c| c.id == "three").unwrap();
    assert_eq!(three_c.reason, "shifted");

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
fn added_and_removed_segments_are_named() {
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
fn reordering_two_unchanged_segments_is_drift() {
    let a = seg("one", 0, 1000, "First.", "a");
    let mut b = seg("two", 1000, 1000, "Second.", "b");
    let before = manifest(vec![a.clone(), b.clone()]);

    b.start_ms = 0;
    let mut a2 = a.clone();
    a2.start_ms = 1000;
    let after = manifest(vec![b, a2]);

    let d = diff(&before, &after);
    assert!(
        d.reordered,
        "order is part of the contract, not an accident"
    );
    assert!(!d.is_empty());
}

#[test]
fn the_report_names_the_duration_change_and_the_segments() {
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
fn an_estimated_segment_that_becomes_measured_is_drift() {
    let mut before = seg("welcome", 0, 2750, "One two three four five six.", "a");
    before.duration_source = "estimated".to_string();
    let after = seg("welcome", 0, 2750, "One two three four five six.", "a");

    let d = diff(&manifest(vec![before]), &manifest(vec![after]));
    assert_eq!(d.changed.len(), 1, "{:?}", d.changed);
    assert_eq!(d.changed[0].reason, "now measured");
}

/// The reverse — a cleared cache turning a measurement back into a
/// prediction — is still drift, but it must not claim something was just
/// measured. Mirrors the timeline diff's own reading of the same
/// transition.
#[test]
fn a_measured_segment_that_reverts_to_estimated_does_not_claim_a_measurement() {
    let before = seg("welcome", 0, 2750, "One two three four five six.", "a");
    let mut after = seg("welcome", 0, 2750, "One two three four five six.", "a");
    after.duration_source = "estimated".to_string();

    let d = diff(&manifest(vec![before]), &manifest(vec![after]));
    assert_ne!(
        d.changed.first().map(|c| c.reason.as_str()),
        Some("now measured")
    );
}

/// `now measured` sits after `text edited`, so an author who rewrote a
/// paragraph is told about their own edit rather than about the cache.
#[test]
fn a_text_edit_outranks_the_measurement_transition() {
    let mut before = seg("welcome", 0, 2750, "One two three four five six.", "a");
    before.duration_source = "estimated".to_string();
    let after = seg("welcome", 0, 3000, "One two three four five six more.", "b");

    let d = diff(&manifest(vec![before]), &manifest(vec![after]));
    assert_eq!(d.changed[0].reason, "text edited");
}

/// And before `audio changed`, so the transition is named rather than being
/// absorbed into a report that sends the reader hunting for a content
/// change that did not happen.
#[test]
fn the_measurement_transition_outranks_a_length_change() {
    let mut before = seg("welcome", 0, 2750, "One two three four five six.", "a");
    before.duration_source = "estimated".to_string();
    let after = seg("welcome", 0, 5000, "One two three four five six.", "b");

    let d = diff(&manifest(vec![before]), &manifest(vec![after]));
    assert_eq!(d.changed[0].reason, "now measured");
}
