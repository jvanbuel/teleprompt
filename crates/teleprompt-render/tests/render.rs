//! The renderer against real ffmpeg.
//!
//! The argv tests in `ffmpeg.rs` say what graph teleprompt builds; only
//! this one says whether ffmpeg accepts it. A graph that is plausible and
//! invalid passes every assertion in the other file.
//!
//! Skipped, loudly, where there is no ffmpeg: the default test run is not
//! allowed to require one, and a silent skip is how a suite stops testing
//! anything without telling you.

use std::path::PathBuf;
use std::process::Command;

use teleprompt_render::ffmpeg::FfmpegRenderer;
use teleprompt_render::{Beat, Narration, Picture, RenderPlan, Renderer, Transition};

fn have_ffmpeg() -> bool {
    let present = Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok();
    // A skip that nobody sees is a test that stopped running. CI sets this
    // and installs ffmpeg, so a missing one there is a broken workflow
    // rather than a machine without a renderer.
    assert!(
        present || std::env::var_os("TELEPROMPT_REQUIRE_FFMPEG").is_none(),
        "TELEPROMPT_REQUIRE_FFMPEG is set and there is no ffmpeg on PATH"
    );
    present
}

/// Seconds of **picture** in `path`, counted frame by frame.
///
/// Not `format=duration`: a container reports the longest stream it holds,
/// so a one-second picture under a three-second audio bed reads back as
/// three seconds and a video that stops early looks correct.
fn duration_of(path: &std::path::Path) -> f64 {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames,avg_frame_rate",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    let csv = String::from_utf8_lossy(&out.stdout);
    let mut fields = csv.trim().split(',');
    let rate = fields.next().expect("a frame rate");
    let frames: f64 = fields
        .next()
        .expect("a frame count")
        .parse()
        .expect("a numeric frame count");
    let (num, den) = rate.split_once('/').expect("a rational frame rate");
    frames * den.parse::<f64>().unwrap() / num.parse::<f64>().unwrap()
}

/// A mono 24 kHz WAV of `ms` milliseconds of quiet tone, written by ffmpeg
/// so the fixture does not depend on the voice crates.
fn tone(dir: &std::path::Path, name: &str, ms: u64) -> PathBuf {
    let path = dir.join(name);
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-y", "-f", "lavfi", "-t"])
        .arg(format!("{}.{:03}", ms / 1000, ms % 1000))
        .args(["-i", "sine=frequency=220:sample_rate=24000", "-ac", "1"])
        .arg(&path)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    path
}

#[test]
fn a_plan_renders_to_a_file_whose_length_is_the_length_it_asked_for() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }

    let dir = std::env::temp_dir().join(format!("tp-render-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plan = RenderPlan {
        width: 640,
        height: 360,
        fps: 24,
        duration_ms: 3_000,
        beats: vec![
            Beat {
                id: "a#0".into(),
                start_ms: 0,
                duration_ms: 1_500,
                picture: Picture::Slate,
                transition: Transition::cut(),
            },
            Beat {
                id: "b#0".into(),
                start_ms: 1_500,
                duration_ms: 1_500,
                picture: Picture::Slate,
                transition: Transition::cut(),
            },
        ],
        narration: vec![Narration {
            id: "one".into(),
            path: tone(&dir, "one.wav", 800),
            start_ms: 1_000,
        }],
        output: dir.join("out.mp4"),
    };

    let rendered = FfmpegRenderer::default()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    assert!(rendered.path.exists());
    let seconds = duration_of(&rendered.path);
    assert!(
        (seconds - 3.0).abs() < 0.2,
        "a 3s plan rendered {seconds}s of video"
    );
}

/// Where the audio actually lands in the file, measured rather than
/// asserted from the argv that asked for it. `adelay` taking the right
/// number is not the same claim as the mix putting the sound there, and
/// only one of the two is what an author hears.
#[test]
fn narration_is_audible_at_the_offset_it_was_placed_at() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }

    let dir = std::env::temp_dir().join(format!("tp-render-audio-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plan = RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms: 4_000,
        beats: vec![Beat {
            id: "only#0".into(),
            start_ms: 0,
            duration_ms: 4_000,
            picture: Picture::Slate,
            transition: Transition::cut(),
        }],
        narration: vec![Narration {
            id: "late".into(),
            path: tone(&dir, "late.wav", 1_000),
            start_ms: 2_000,
        }],
        output: dir.join("late.mp4"),
    };

    FfmpegRenderer::default()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    let onset = first_sound(&plan.output);
    assert!(
        (onset - 2.0).abs() < 0.15,
        "narration placed at 2.000s was first audible at {onset}s"
    );
}

/// Seconds at which `path` stops being silent, per ffmpeg's own
/// `silencedetect`.
fn first_sound(path: &std::path::Path) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-i"])
        .arg(path)
        .args(["-af", "silencedetect=noise=-50dB:d=0.1", "-f", "null", "-"])
        .output()
        .expect("ffmpeg runs");
    let log = String::from_utf8_lossy(&out.stderr);
    log.lines()
        .find_map(|l| l.split("silence_end: ").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no sound at all in the render:\n{log}"))
}

/// The picture does not begin at zero. Narration opens after a lead-in, and
/// the first beat starts with it — so the head of the video is a gap that
/// something has to hold, or every beat after it renders early against
/// audio that is still correctly placed.
#[test]
fn a_gap_before_the_first_beat_is_held_rather_than_closed() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }

    let dir = std::env::temp_dir().join(format!("tp-render-gap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plan = RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms: 3_000,
        beats: vec![Beat {
            id: "late#0".into(),
            start_ms: 500,
            duration_ms: 1_000,
            picture: Picture::Slate,
            transition: Transition::cut(),
        }],
        narration: vec![Narration {
            id: "one".into(),
            path: tone(&dir, "one.wav", 500),
            start_ms: 500,
        }],
        output: dir.join("gap.mp4"),
    };

    FfmpegRenderer::default()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    let seconds = duration_of(&plan.output);
    assert!(
        (seconds - 3.0).abs() < 0.2,
        "the video runs as long as the plan, head gap and tail included, \
         not just as long as its beats: {seconds}s"
    );
}

/// A crossfade costs time: two beats joined by one occupy less of the
/// timeline than they do apart, which is why the scheduler overlaps them.
/// A renderer that concatenated them instead would run the same frames for
/// half a second longer and put every later beat out of step with its
/// narration.
#[test]
fn a_crossfade_overlaps_the_beats_it_joins() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }

    let dir = std::env::temp_dir().join(format!("tp-render-xfade-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plan = RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms: 3_500,
        beats: vec![
            Beat {
                id: "a#0".into(),
                start_ms: 0,
                duration_ms: 2_000,
                picture: Picture::Slate,
                transition: Transition {
                    kind: "crossfade".into(),
                    duration_ms: 500,
                },
            },
            Beat {
                id: "b#0".into(),
                // 2_000 - 500: the scheduler already subtracted the fade.
                start_ms: 1_500,
                duration_ms: 2_000,
                picture: Picture::Slate,
                transition: Transition::cut(),
            },
        ],
        narration: vec![],
        output: dir.join("xfade.mp4"),
    };

    FfmpegRenderer::default()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    let seconds = duration_of(&plan.output);
    assert!(
        (seconds - 3.5).abs() < 0.15,
        "two 2s beats crossfaded by 0.5s make 3.5s of picture, not {seconds}s"
    );
}

/// A script can be prose alone — no action blocks, nothing to show. It
/// still has a length, and a video of it is a legitimate thing to ask for.
#[test]
fn a_plan_with_no_beats_at_all_still_renders_its_narration() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = std::env::temp_dir().join(format!("tp-render-prose-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plan = RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms: 2_000,
        beats: vec![],
        narration: vec![Narration {
            id: "alone".into(),
            path: tone(&dir, "alone.wav", 1_000),
            start_ms: 200,
        }],
        output: dir.join("prose.mp4"),
    };

    FfmpegRenderer::default()
        .render(&plan, &mut |_| {})
        .expect("a script with no action blocks is still a script");
    assert!((duration_of(&plan.output) - 2.0).abs() < 0.2);
}

/// A beat of no length is a beat there is nothing to show for, and
/// `trim=duration=0` is not something ffmpeg will accept. It is dropped
/// rather than rendered, which changes nothing about the timeline: the
/// piece around it already covers those zero milliseconds.
#[test]
fn a_beat_of_no_length_does_not_reach_the_graph() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = std::env::temp_dir().join(format!("tp-render-zero-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plan = RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms: 2_000,
        beats: vec![
            Beat {
                id: "empty#0".into(),
                start_ms: 0,
                duration_ms: 0,
                picture: Picture::Slate,
                transition: Transition::cut(),
            },
            Beat {
                id: "real#0".into(),
                start_ms: 0,
                duration_ms: 2_000,
                picture: Picture::Slate,
                transition: Transition::cut(),
            },
        ],
        narration: vec![],
        output: dir.join("zero.mp4"),
    };

    FfmpegRenderer::default()
        .render(&plan, &mut |_| {})
        .expect("a zero-length beat is dropped, not rendered");
    assert!((duration_of(&plan.output) - 2.0).abs() < 0.2);
}

/// A captured clip is fitted to the slot the scheduler gave it, in both
/// directions: a short one holds its last frame, a long one is cut. The
/// alternative is a clip deciding the timeline, which is the thing this
/// tool exists not to do.
#[test]
fn a_clip_is_fitted_to_the_slot_rather_than_the_slot_to_the_clip() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = std::env::temp_dir().join(format!("tp-render-clip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // A one-second clip, standing in for something a capture stage will
    // one day produce.
    let clip = dir.join("captured.mp4");
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-y",
            "-f",
            "lavfi",
            "-t",
            "1",
            "-i",
            "testsrc=size=640x360:rate=30",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&clip)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());

    let plan = RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms: 3_000,
        beats: vec![
            Beat {
                id: "short#0".into(),
                start_ms: 0,
                duration_ms: 2_000,
                picture: Picture::Clip(clip.clone()),
                transition: Transition::cut(),
            },
            Beat {
                id: "long#0".into(),
                start_ms: 2_000,
                duration_ms: 500,
                picture: Picture::Clip(clip),
                transition: Transition::cut(),
            },
        ],
        narration: vec![],
        output: dir.join("clips.mp4"),
    };

    FfmpegRenderer::default()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    // 2s held + 0.5s cut + 0.5s of tail hold to the plan's length.
    assert!((duration_of(&plan.output) - 3.0).abs() < 0.15);
}
