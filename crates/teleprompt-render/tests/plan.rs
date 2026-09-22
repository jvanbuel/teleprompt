//! Turning a published manifest into a render plan.
//!
//! The manifest is parsed from JSON here rather than built from the
//! compiler's own types, because that is what the seam is: `build` reads
//! the artifact an outside consumer reads, and a test that constructs the
//! struct in memory would not notice the day the two stop matching.

use std::path::{Path, PathBuf};

use teleprompt_compile::manifest::NarrationManifest;
use teleprompt_render::plan::{self, Inputs};
use teleprompt_render::Picture;

const MANIFEST: &str = r#"{
  "manifest_version": 2,
  "script": "tour.md",
  "locale": "en",
  "generated_by": "teleprompt 0.1.0",
  "duration_ms": 4300,
  "audio": { "format": "wav", "sample_rate": 24000, "channels": 1 },
  "chapters": [{ "id": "opening", "title": "Opening", "start_ms": 150 }],
  "lines": [
    {
      "id": "welcome",
      "text": "Welcome.",
      "chapter": "opening",
      "start_ms": 150,
      "duration_ms": 2000,
      "duration_source": "measured",
      "audio": "audio/welcome.wav",
      "voice_source": "synthetic",
      "voice_source_actual": "synthetic",
      "downgrade_reason": null,
      "source_hash": "1111111111111111111111111111111111111111111111111111111111111111",
      "audio_hash": "2222222222222222222222222222222222222222222222222222222222222222"
    }
  ],
  "cues": [
    {
      "cue": "plan#0",
      "line": "welcome",
      "scene": "terminal",
      "adapter": "vhs",
      "start_ms": 150,
      "duration_ms": 2000,
      "duration_source": "exact",
      "policy": "hold",
      "transition": { "kind": "cut", "duration_ms": 0 },
      "cue_hash": "3333333333333333333333333333333333333333333333333333333333333333",
      "capture_key": "5555555555555555555555555555555555555555555555555555555555555555"
    },
    {
      "cue": "plan#1",
      "line": null,
      "scene": "terminal",
      "adapter": "vhs",
      "start_ms": 2150,
      "duration_ms": 2000,
      "duration_source": "exact",
      "policy": "concurrent",
      "transition": { "kind": "cut", "duration_ms": 0 },
      "cue_hash": "4444444444444444444444444444444444444444444444444444444444444444",
      "capture_key": "6666666666666666666666666666666666666666666666666666666666666666"
    }
  ]
}"#;

fn inputs(dir: &Path) -> Inputs {
    Inputs {
        narration_dir: dir.join("narration/en"),
        clips_dir: dir.join("cache/video"),
        output: dir.join("out/tour.mp4"),
        width: 1920,
        height: 1080,
        fps: 30,
    }
}

fn manifest() -> NarrationManifest {
    serde_json::from_str(MANIFEST).expect("the fixture is a v2 manifest")
}

#[test]
fn the_plan_places_narration_where_the_manifest_placed_it() {
    let dir = PathBuf::from("/project");
    let (plan, _) = plan::from_manifest(&manifest(), &inputs(&dir));

    assert_eq!(plan.duration_ms, 4_300);
    assert_eq!(plan.narration.len(), 1);
    assert_eq!(plan.narration[0].id, "welcome");
    assert_eq!(plan.narration[0].start_ms, 150);
    assert_eq!(
        plan.narration[0].path,
        dir.join("narration/en/audio/welcome.wav"),
        "audio paths in a manifest are relative to the manifest"
    );
}

#[test]
fn a_beat_with_no_captured_clip_is_a_slate_and_is_reported() {
    let dir = PathBuf::from("/project");
    let (plan, warnings) = plan::from_manifest(&manifest(), &inputs(&dir));

    assert_eq!(plan.cues.len(), 2);
    assert!(plan.cues.iter().all(|b| b.picture == Picture::Slate));
    assert_eq!(
        warnings.len(),
        1,
        "one line for the lot, not one per cue: {warnings:?}"
    );
    assert!(
        warnings[0].starts_with("2 of 2 cue(s) have no captured clip"),
        "the warning counts what is missing: {warnings:?}"
    );
}

#[test]
fn a_captured_clip_is_used_where_one_exists_for_the_span() {
    let dir = std::env::temp_dir().join(format!("tp-plan-{}", std::process::id()));
    let clips = dir.join("cache/video");
    std::fs::create_dir_all(&clips).unwrap();
    let hash = "5555555555555555555555555555555555555555555555555555555555555555";
    let clip = clips.join(format!("{hash}.mp4"));
    std::fs::write(&clip, b"not really an mp4").unwrap();

    let (plan, warnings) = plan::from_manifest(&manifest(), &inputs(&dir));

    assert_eq!(
        plan.cues[0].picture,
        Picture::Clip(clip),
        "a clip is found by the capture key the manifest published"
    );
    assert_eq!(plan.cues[1].picture, Picture::Slate);
    assert!(
        warnings[0].starts_with("1 of 2 cue(s) have no captured clip"),
        "and only the uncaptured cue is reported: {warnings:?}"
    );
}

/// A pause is a cue during which the picture holds — that is what a pause
/// is. Looking for a clip under its hash would find nothing, count it as
/// uncaptured, and cut to a blank field in the middle of a script that is
/// working perfectly.
#[test]
fn a_pause_holds_the_picture_rather_than_asking_for_a_clip() {
    let with_pause = MANIFEST.replace(
        r#"      "scene": "terminal",
      "adapter": "vhs",
      "start_ms": 2150,"#,
        r#"      "scene": "pause",
      "adapter": "pause",
      "start_ms": 2150,"#,
    );
    let manifest: NarrationManifest = serde_json::from_str(&with_pause).unwrap();
    let (plan, warnings) = plan::from_manifest(&manifest, &inputs(&PathBuf::from("/project")));

    assert_eq!(plan.cues[1].picture, Picture::Hold);
    assert!(
        warnings[0].starts_with("1 of 2 cue(s) have no captured clip"),
        "a pause is not a missing capture: {warnings:?}"
    );
}
