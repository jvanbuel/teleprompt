//! The ffmpeg renderer, tested through the argument vector it builds.
//!
//! Asserting on the argv rather than on a rendered file is what keeps the
//! default test run free of ffmpeg: the graph is the part teleprompt owns,
//! and the part that can be wrong in ways nobody notices until the audio
//! drifts a second late halfway through a long video.

use std::path::PathBuf;

use teleprompt_render::ffmpeg;
use teleprompt_render::{Narration, Picture, RenderPlan, Shot, Transition};

fn plan() -> RenderPlan {
    RenderPlan {
        width: 1920,
        height: 1080,
        fps: 30,
        duration_ms: 4_000,
        shots: vec![Shot {
            id: "intro#0".into(),
            start_ms: 0,
            duration_ms: 4_000,
            picture: Picture::Slate,
            transition: Transition::cut(),
        }],
        narration: vec![Narration {
            id: "welcome".into(),
            path: PathBuf::from("/cache/welcome.wav"),
            start_ms: 150,
        }],
        output: PathBuf::from("/out/tour.mp4"),
    }
}

/// The offsets are the whole job. A narration clip belongs where the
/// manifest says it does, in milliseconds, and `adelay` is what puts it
/// there.
#[test]
fn a_narration_clip_is_delayed_to_the_offset_the_manifest_published() {
    let args = ffmpeg::args(&plan());
    let joined = args.join(" ");

    assert!(
        joined.contains("/cache/welcome.wav"),
        "the clip is an input: {joined}"
    );
    assert!(
        joined.contains("adelay=150|150"),
        "and it is delayed to its published offset: {joined}"
    );
}

/// A shot with nothing captured still occupies its slot. Skipping it would
/// run everything after it early — the picture would drift against speech
/// that is still correctly placed.
#[test]
fn a_shot_with_no_capture_holds_its_slot_with_a_slate() {
    let args = ffmpeg::args(&plan());
    let joined = args.join(" ");

    assert!(
        joined.contains("color=") && joined.contains("1920x1080"),
        "the slate is a colour source at the output size: {joined}"
    );
    assert!(
        joined.contains("-t 4.000"),
        "and it lasts exactly as long as the shot was scheduled for: {joined}"
    );
    assert!(
        joined.contains("libx264") && joined.contains("yuv420p"),
        "the output carries a video stream: {joined}"
    );
}
