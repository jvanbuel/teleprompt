use std::path::Path;

use teleprompt_compile::manifest::{self, AudioInfo, MANIFEST_VERSION};
use teleprompt_compile::{compile, CompileOutput};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Program};
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::NullVoice;

fn program_for(src: &str) -> Program {
    let mut parsed = parse_script(src).expect("fixture parses");
    let diags = assign_ids(&mut parsed);
    assert!(
        !diags.iter().any(|d| d.is_error()),
        "fixture must yield unambiguous segment ids"
    );
    resolve(
        &parsed,
        "tour.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect("fixture resolves")
}

fn compiled_and_manifest(src: &str) -> (CompileOutput, manifest::NarrationManifest) {
    let program = program_for(src);
    let out = compile(
        &program,
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
        Path::new("."),
        "0.1.0",
    )
    .unwrap();
    let m = manifest::build(
        &out.timeline,
        &program.chapters,
        &out.narration,
        AudioInfo {
            format: "wav".to_string(),
            sample_rate: 48_000,
            channels: 1,
        },
    );
    (out, m)
}

fn manifest_for(src: &str) -> manifest::NarrationManifest {
    let program = program_for(src);
    let out = compile(
        &program,
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
        Path::new("."),
        "0.1.0",
    )
    .unwrap();
    manifest::build(
        &out.timeline,
        &program.chapters,
        &out.narration,
        AudioInfo {
            format: "wav".to_string(),
            sample_rate: 48_000,
            channels: 1,
        },
    )
}

const TWO_CHAPTERS: &str = "\
# Quick start

Every video in this repository is built from a script you can read.

# Provenance

And every timeline is committed alongside it.
";

#[test]
fn segment_timings_equal_the_timeline_they_came_from() {
    let (out, m) = compiled_and_manifest(TWO_CHAPTERS);

    let from_timeline: Vec<(String, u64, u64)> = out
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .map(|n| (n.segment.clone(), n.start_ms, n.duration_ms))
        .collect();
    let from_manifest: Vec<(String, u64, u64)> = m
        .segments
        .iter()
        .map(|s| (s.id.clone(), s.start_ms, s.duration_ms))
        .collect();

    assert_eq!(
        from_timeline, from_manifest,
        "the manifest must not be able to disagree with the schedule it describes"
    );
    assert_eq!(m.duration_ms, out.timeline.duration_ms);
}

#[test]
fn chapters_carry_the_start_of_their_first_spoken_segment() {
    let m = manifest_for(TWO_CHAPTERS);
    assert_eq!(m.chapters.len(), 2);
    assert_eq!(m.chapters[0].id, "quick-start");
    assert_eq!(m.chapters[0].title, "Quick start");
    assert_eq!(m.chapters[0].start_ms, m.segments[0].start_ms);
    assert_eq!(m.chapters[1].id, "provenance");
    assert_eq!(m.chapters[1].start_ms, m.segments[1].start_ms);
}

#[test]
fn a_chapter_with_no_narration_is_omitted() {
    let m = manifest_for(
        "\
# Intro

Only this chapter speaks.

# Silent
",
    );
    let ids: Vec<&str> = m.chapters.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["intro"],
        "a chapter marker with no start time would be meaningless to a consumer"
    );
}

#[test]
fn audio_paths_are_relative_to_the_manifest() {
    let m = manifest_for(TWO_CHAPTERS);
    assert_eq!(
        m.segments[0].audio,
        format!("audio/{}.wav", m.segments[0].id)
    );
    assert!(
        !m.segments[0].audio.starts_with('/'),
        "an absolute path would not survive being served from anywhere else"
    );
}

#[test]
fn header_records_version_provenance_and_audio_shape() {
    let m = manifest_for(TWO_CHAPTERS);
    assert_eq!(m.manifest_version, MANIFEST_VERSION);
    assert_eq!(m.script, "tour.md");
    assert_eq!(m.locale, "en");
    assert_eq!(m.generated_by, "teleprompt 0.1.0");
    assert_eq!(m.audio.format, "wav");
    assert_eq!(m.audio.sample_rate, 48_000);
    assert_eq!(m.audio.channels, 1);
}

#[test]
fn downgrade_reason_is_present_as_null_but_words_are_omitted() {
    let m = manifest_for(TWO_CHAPTERS);
    let json = serde_json::to_value(&m).unwrap();
    let seg = &json["segments"][0];

    assert!(
        seg.get("downgrade_reason").is_some(),
        "consumers should not have to distinguish absent from null here"
    );
    assert!(seg["downgrade_reason"].is_null());
    assert!(
        seg.get("words").is_none(),
        "absent means the backend has no word timings; an empty array would be a lie"
    );
}

#[test]
fn the_manifest_json_shape_is_stable() {
    let m = manifest_for(TWO_CHAPTERS);
    insta::assert_json_snapshot!(m);
}

#[test]
fn serialization_is_byte_stable() {
    let a = serde_json::to_string_pretty(&manifest_for(TWO_CHAPTERS)).unwrap();
    let b = serde_json::to_string_pretty(&manifest_for(TWO_CHAPTERS)).unwrap();
    assert_eq!(a, b);
}
