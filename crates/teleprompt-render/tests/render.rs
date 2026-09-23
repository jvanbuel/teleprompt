//! The renderer against real ffmpeg.
//!
//! The argv tests in `ffmpeg.rs` say what graph teleprompt builds; only
//! this one says whether ffmpeg accepts it. A graph that is plausible and
//! invalid passes every assertion in the other file.
//!
//! Skipped, loudly, where there is no ffmpeg: the default test run is not
//! allowed to require one, and a silent skip is how a suite stops testing
//! anything without telling you.

use std::process::Command;

use teleprompt_render::incremental::IncrementalRenderer;
use teleprompt_render::{Narration, Picture, RenderPlan, Renderer, Shot, Transition};

mod support;
use support::{bright_clip, duration_of, first_sound, have_ffmpeg, luma_at, tone};

/// The renderer with reuse switched off — what `--no-cache` asks for, and
/// what these tests want: every frame encoded here, nothing served from a
/// cache a previous test filled.
fn one_pass() -> IncrementalRenderer {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    IncrementalRenderer {
        program: "ffmpeg".into(),
        // A directory of its own per call. These run in parallel in one
        // process, and a chunk key is content-addressed, so two tests
        // rendering similar plans write the *same* filename — which is a
        // half-written mp4 and `moov atom not found`.
        cache_dir: std::env::temp_dir().join(format!(
            "tp-onepass-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )),
        reuse: false,
    }
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
        shots: vec![
            Shot {
                id: "a#0".into(),
                start_ms: 0,
                duration_ms: 1_500,
                picture: Picture::Slate,
                transition: Transition::cut(),
            },
            Shot {
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

    let rendered = one_pass()
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
        shots: vec![Shot {
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

    one_pass()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    let onset = first_sound(&plan.output);
    assert!(
        (onset - 2.0).abs() < 0.15,
        "narration placed at 2.000s was first audible at {onset}s"
    );
}

/// The voice's rate is not the video's. Kokoro speaks mono at 24 kHz —
/// as `tone` does — and AAC at that rate is valid and silent in more than
/// one player, which is a video with a picture and none of its words. What
/// ships is 48 kHz stereo, whatever went in.
#[test]
fn the_audio_is_48_khz_stereo_whatever_the_voice_spoke() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }

    let dir = std::env::temp_dir().join(format!("tp-render-rate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plan = RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms: 1_000,
        shots: vec![Shot {
            id: "only#0".into(),
            start_ms: 0,
            duration_ms: 1_000,
            picture: Picture::Slate,
            transition: Transition::cut(),
        }],
        narration: vec![Narration {
            id: "voice".into(),
            path: tone(&dir, "voice.wav", 500),
            start_ms: 0,
        }],
        output: dir.join("rate.mp4"),
    };

    one_pass()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a:0"])
        .args(["-show_entries", "stream=sample_rate,channels"])
        .args(["-of", "default=nw=1"])
        .arg(&plan.output)
        .output()
        .expect("ffprobe runs");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("sample_rate=48000"), "{said}");
    assert!(said.contains("channels=2"), "{said}");
}

/// The picture does not begin at zero. Narration opens after a lead-in, and
/// the first shot starts with it — so the head of the video is a gap that
/// something has to hold, or every shot after it renders early against
/// audio that is still correctly placed.
#[test]
fn a_gap_before_the_first_shot_is_held_rather_than_closed() {
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
        shots: vec![Shot {
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

    one_pass()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    let seconds = duration_of(&plan.output);
    assert!(
        (seconds - 3.0).abs() < 0.2,
        "the video runs as long as the plan, head gap and tail included, \
         not just as long as its shots: {seconds}s"
    );
}

/// A crossfade costs time: two shots joined by one occupy less of the
/// timeline than they do apart, which is why the scheduler overlaps them.
/// A renderer that concatenated them instead would run the same frames for
/// half a second longer and put every later shot out of step with its
/// narration.
#[test]
fn a_crossfade_overlaps_the_shots_it_joins() {
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
        shots: vec![
            Shot {
                id: "a#0".into(),
                start_ms: 0,
                duration_ms: 2_000,
                picture: Picture::Slate,
                transition: Transition {
                    kind: "crossfade".into(),
                    duration_ms: 500,
                },
            },
            Shot {
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

    one_pass()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    let seconds = duration_of(&plan.output);
    assert!(
        (seconds - 3.5).abs() < 0.15,
        "two 2s shots crossfaded by 0.5s make 3.5s of picture, not {seconds}s"
    );
}

/// A script can be prose alone — no action blocks, nothing to show. It
/// still has a length, and a video of it is a legitimate thing to ask for.
#[test]
fn a_plan_with_no_shots_at_all_still_renders_its_narration() {
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
        shots: vec![],
        narration: vec![Narration {
            id: "alone".into(),
            path: tone(&dir, "alone.wav", 1_000),
            start_ms: 200,
        }],
        output: dir.join("prose.mp4"),
    };

    one_pass()
        .render(&plan, &mut |_| {})
        .expect("a script with no action blocks is still a script");
    assert!((duration_of(&plan.output) - 2.0).abs() < 0.2);
}

/// A shot of no length is a shot there is nothing to show for, and
/// `trim=duration=0` is not something ffmpeg will accept. It is dropped
/// rather than rendered, which changes nothing about the timeline: the
/// placement around it already covers those zero milliseconds.
#[test]
fn a_shot_of_no_length_does_not_reach_the_graph() {
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
        shots: vec![
            Shot {
                id: "empty#0".into(),
                start_ms: 0,
                duration_ms: 0,
                picture: Picture::Slate,
                transition: Transition::cut(),
            },
            Shot {
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

    one_pass()
        .render(&plan, &mut |_| {})
        .expect("a zero-length shot is dropped, not rendered");
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
        shots: vec![
            Shot {
                id: "short#0".into(),
                start_ms: 0,
                duration_ms: 2_000,
                picture: Picture::Clip(clip.clone()),
                transition: Transition::cut(),
            },
            Shot {
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

    one_pass()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    // 2s held + 0.5s cut + 0.5s of tail hold to the plan's length.
    assert!((duration_of(&plan.output) - 3.0).abs() < 0.15);
}

/// Narration keeps talking after the action stops — most of a script is
/// like this — and the picture has to do something during it. Cutting to a
/// slate makes a video that is mostly black while somebody speaks over it.
/// It holds the last frame instead.
#[test]
fn a_gap_after_a_shot_freezes_its_last_frame() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = std::env::temp_dir().join(format!("tp-render-freeze-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plan = RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms: 6_000,
        shots: vec![Shot {
            id: "only#0".into(),
            start_ms: 0,
            duration_ms: 1_000,
            picture: Picture::Clip(bright_clip(&dir, "bright.mp4")),
            transition: Transition::cut(),
        }],
        narration: vec![],
        output: dir.join("freeze.mp4"),
    };

    one_pass()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    for t in [2.0, 4.0, 5.5] {
        let luma = luma_at(&plan.output, t);
        assert!(
            luma > 100.0,
            "the picture went dark {t}s in, during narration: YAVG {luma}"
        );
    }
}

/// The same at the head. Narration opens after a lead-in and the first shot
/// starts with it, so a video that cut to black until then would open on
/// black every single time.
#[test]
fn a_gap_before_the_first_shot_holds_its_first_frame() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = std::env::temp_dir().join(format!("tp-render-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plan = RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms: 4_000,
        shots: vec![Shot {
            id: "late#0".into(),
            start_ms: 2_000,
            duration_ms: 1_000,
            picture: Picture::Clip(bright_clip(&dir, "bright.mp4")),
            transition: Transition::cut(),
        }],
        narration: vec![],
        output: dir.join("open.mp4"),
    };

    one_pass()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    let luma = luma_at(&plan.output, 0.5);
    assert!(
        luma > 100.0,
        "the video opened on black before its first shot: YAVG {luma}"
    );
    assert!((duration_of(&plan.output) - 4.0).abs() < 0.2);
}

/// A shot that contributes no picture must not take the gap in front of it
/// with it. Dropping a zero-length shot used to drop the hold that preceded
/// it, and the video came out a minute shorter than its own timeline.
#[test]
fn dropping_an_empty_shot_does_not_drop_the_time_before_it() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = std::env::temp_dir().join(format!("tp-render-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let plan = RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms: 6_000,
        shots: vec![
            Shot {
                id: "real#0".into(),
                start_ms: 0,
                duration_ms: 1_000,
                picture: Picture::Clip(bright_clip(&dir, "bright.mp4")),
                transition: Transition::cut(),
            },
            Shot {
                id: "empty#0".into(),
                // Three seconds of narration sit between the two.
                start_ms: 4_000,
                duration_ms: 0,
                picture: Picture::Slate,
                transition: Transition::cut(),
            },
        ],
        narration: vec![],
        output: dir.join("empty.mp4"),
    };

    one_pass()
        .render(&plan, &mut |_| {})
        .expect("the graph is one ffmpeg accepts");

    let seconds = duration_of(&plan.output);
    assert!(
        (seconds - 6.0).abs() < 0.2,
        "a 6s plan rendered {seconds}s of picture"
    );
    assert!(
        luma_at(&plan.output, 5.0) > 100.0,
        "and it is still holding"
    );
}
