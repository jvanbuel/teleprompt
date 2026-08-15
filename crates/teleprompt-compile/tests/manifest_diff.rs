use teleprompt_compile::manifest::{AudioInfo, NarrationManifest, SegmentEntry, MANIFEST_VERSION};
use teleprompt_compile::manifest_diff::diff;
use teleprompt_core::Hash;

fn seg(id: &str, start_ms: u64, duration_ms: u64, text: &str, audio_seed: &str) -> SegmentEntry {
    SegmentEntry {
        id: id.to_string(),
        text: text.to_string(),
        start_ms,
        duration_ms,
        audio: format!("audio/{id}.wav"),
        voice_source: "synthetic".to_string(),
        voice_source_actual: "synthetic".to_string(),
        downgrade_reason: None,
        source_hash: Hash::of(text.as_bytes()),
        audio_hash: Hash::of(audio_seed.as_bytes()),
        words: None,
    }
}

fn manifest(segments: Vec<SegmentEntry>) -> NarrationManifest {
    let duration_ms = segments
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
        segments,
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
fn a_segment_that_only_moved_is_still_drift() {
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![seg("welcome", 500, 1000, "Hello.", "a")]);
    let d = diff(&before, &after);

    assert!(
        !d.is_empty(),
        "a moved segment desynchronises every consumer"
    );
    assert_eq!(d.changed[0].reason, "moved");
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
