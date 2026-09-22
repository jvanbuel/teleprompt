//! The incremental renderer against real ffmpeg.
//!
//! Two claims to hold down. The first is that cutting a video into cached
//! segments and copying them back together produces the same video — same
//! length, same picture, narration in the same place. The second is that
//! the cache actually skips work: a rebuild after a small edit has to
//! re-encode the part that moved and nothing else, and the only honest way
//! to check that is to look at which files the renderer touched.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use teleprompt_render::ffmpeg::FfmpegRenderer;
use teleprompt_render::incremental::IncrementalRenderer;
use teleprompt_render::{Beat, Narration, Picture, RenderPlan, Renderer, Transition};

mod support;
use support::{bright_clip, duration_of, first_sound, have_ffmpeg, luma_at, workdir};

fn renderer(cache: &Path) -> IncrementalRenderer {
    IncrementalRenderer {
        program: "ffmpeg".into(),
        cache_dir: cache.to_path_buf(),
    }
}

/// Every cached segment.
fn cache_state(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("the cache directory exists")
        .map(|e| e.expect("a readable entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "mp4"))
        .collect();
    entries.sort();
    entries
}

/// When a cached segment was last used.
fn last_used(path: &Path) -> SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

fn plan(dir: &Path, clip: &Path, beats: &[(u64, u64)], duration_ms: u64) -> RenderPlan {
    RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms,
        beats: beats
            .iter()
            .enumerate()
            .map(|(i, (start_ms, duration_ms))| Beat {
                id: format!("b{i}#0"),
                start_ms: *start_ms,
                duration_ms: *duration_ms,
                picture: Picture::Clip(clip.to_path_buf()),
                transition: Transition::cut(),
            })
            .collect(),
        narration: Vec::new(),
        output: dir.join("out.mp4"),
    }
}

/// The whole point of the exercise: the incremental path and the one-pass
/// path are the same video. A faster renderer that produces a different
/// file is not a faster renderer.
#[test]
fn an_incremental_render_is_the_same_length_as_a_one_pass_render() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-same");
    let clip = bright_clip(&dir, "clip.mp4");

    let mut whole = plan(&dir, &clip, &[(0, 2_000), (2_000, 1_500)], 5_000);
    whole.output = dir.join("whole.mp4");
    FfmpegRenderer::default()
        .render(&whole, &mut |_| {})
        .expect("the one-pass graph renders");

    let mut parts = whole.clone();
    parts.output = dir.join("parts.mp4");
    let rendered = renderer(&dir.join("cache"))
        .render(&parts, &mut |_| {})
        .expect("the incremental graph renders");

    let (a, b) = (duration_of(&whole.output), duration_of(&rendered.path));
    assert!(
        (a - b).abs() < 0.05,
        "one pass rendered {a}s and segments rendered {b}s"
    );
    assert!((b - 5.0).abs() < 0.1, "a 5s plan rendered {b}s");
    assert_eq!(
        rendered.reused_ms,
        Some(0),
        "nothing was cached before this render"
    );
}

/// A gap between beats is held on the last frame, and it is held through
/// the concat too — the seam between two segments must not be a flash of
/// black.
#[test]
fn the_picture_holds_across_the_seam_between_segments() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-seam");
    let clip = bright_clip(&dir, "clip.mp4");
    let plan = plan(&dir, &clip, &[(0, 1_000), (3_000, 1_000)], 5_000);

    renderer(&dir.join("cache"))
        .render(&plan, &mut |_| {})
        .expect("renders");

    for t in [0.5, 1.5, 2.5, 3.5, 4.5] {
        let luma = luma_at(&plan.output, t);
        assert!(luma > 100.0, "the picture went dark {t}s in: YAVG {luma}");
    }
}

/// Narration survives the copy. The picture is concatenated without being
/// decoded; the audio is mixed over the whole of it in one pass, and it
/// still has to land where the manifest put it.
#[test]
fn narration_lands_where_it_was_placed_through_the_concat() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-audio");
    let clip = bright_clip(&dir, "clip.mp4");
    let mut plan = plan(&dir, &clip, &[(0, 1_000), (2_000, 1_000)], 4_000);
    plan.narration = vec![Narration {
        id: "late".into(),
        path: support::tone(&dir, "late.wav", 1_000),
        start_ms: 2_000,
    }];

    renderer(&dir.join("cache"))
        .render(&plan, &mut |_| {})
        .expect("renders");

    let onset = first_sound(&plan.output);
    assert!(
        (onset - 2.0).abs() < 0.15,
        "narration placed at 2.000s was first audible at {onset}s"
    );
}

/// The reason any of this exists. Re-rendering an unchanged plan must
/// encode nothing at all.
#[test]
fn a_second_render_of_the_same_plan_encodes_nothing() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-warm");
    let clip = bright_clip(&dir, "clip.mp4");
    let cache = dir.join("cache");
    let plan = plan(&dir, &clip, &[(0, 1_000), (2_000, 1_000)], 4_000);

    let renderer = renderer(&cache);
    renderer.render(&plan, &mut |_| {}).expect("renders cold");
    let before = cache_state(&cache);
    assert!(!before.is_empty(), "the cold render filled the cache");

    let rendered = renderer.render(&plan, &mut |_| {}).expect("renders warm");

    assert_eq!(
        cache_state(&cache),
        before,
        "a warm render added or dropped a segment"
    );
    assert_eq!(
        rendered.reused_ms,
        Some(4_000),
        "every frame came from the cache"
    );
    assert!((duration_of(&rendered.path) - 4.0).abs() < 0.1);
}

/// And the case that makes it worth having: one beat moves, and the rest
/// of the video is copied rather than encoded. This is what a rebuild after
/// an edit looks like.
#[test]
fn changing_one_beat_re_encodes_only_the_segments_that_moved() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-edit");
    // Three different pictures, because two beats showing the same frames
    // for the same length *are* the same segment — the cache is entitled
    // to encode them once, and does.
    let a = support::colour_clip(&dir, "a.mp4", "0x808080");
    let b = support::colour_clip(&dir, "b.mp4", "0x909090");
    let c = support::colour_clip(&dir, "c.mp4", "0xa0a0a0");
    let edited = support::colour_clip(&dir, "edited.mp4", "0x404040");

    let cache = dir.join("cache");
    let renderer = renderer(&cache);
    let mut plan = plan(
        &dir,
        &a,
        &[(0, 1_000), (2_000, 1_000), (4_000, 1_000)],
        6_000,
    );
    plan.beats[1].picture = Picture::Clip(b);
    plan.beats[2].picture = Picture::Clip(c);

    renderer.render(&plan, &mut |_| {}).expect("renders cold");
    let before = cache_state(&cache);
    assert_eq!(before.len(), 3, "one segment per beat: {before:#?}");

    // The middle beat now shows something else. The beats either side of
    // it did not move — same clip, same slot, same held tail — so their
    // segments are still exactly the frames already on disk.
    plan.beats[1].picture = Picture::Clip(edited);
    let rendered = renderer
        .render(&plan, &mut |_| {})
        .expect("renders after the edit");

    assert_eq!(
        rendered.reused_ms,
        Some(4_000),
        "the two untouched beats — 2s each, held gaps included — came \
         from the cache"
    );
    let after = cache_state(&cache);
    assert_eq!(
        after.len(),
        4,
        "one segment encoded, three kept: {after:#?}"
    );
    assert_eq!(
        after.iter().filter(|e| before.contains(e)).count(),
        3,
        "a segment that did not change was written again:\n{before:#?}\n{after:#?}"
    );
    assert!((duration_of(&rendered.path) - 6.0).abs() < 0.1);
}

/// A transition is a segment of its own, so a plan that has one still has
/// to come out the length the scheduler planned for it.
#[test]
fn a_crossfade_survives_being_a_segment_of_its_own() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-xfade");
    let clip = bright_clip(&dir, "clip.mp4");
    let mut plan = plan(&dir, &clip, &[(0, 2_000), (1_500, 2_000)], 3_500);
    plan.beats[0].transition = Transition {
        kind: "crossfade".into(),
        duration_ms: 500,
    };

    let rendered = renderer(&dir.join("cache"))
        .render(&plan, &mut |_| {})
        .expect("renders");

    let seconds = duration_of(&rendered.path);
    assert!(
        (seconds - 3.5).abs() < 0.1,
        "two 2s beats crossfaded by 0.5s make 3.5s of picture, not {seconds}s"
    );
    for t in [0.5, 1.6, 3.0] {
        let luma = luma_at(&rendered.path, t);
        assert!(luma > 60.0, "the picture went dark {t}s in: YAVG {luma}");
    }
}

/// A plan the segmenter cannot cut is still a plan. It renders in one
/// pass — slower, identical output — rather than failing.
#[test]
fn a_plan_that_cannot_be_cut_still_renders() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-fallback");
    let clip = bright_clip(&dir, "clip.mp4");
    // A 500ms beat with 400ms of blend at each end: the two joins would
    // draw the same frames twice.
    let mut plan = plan(
        &dir,
        &clip,
        &[(0, 2_000), (1_600, 500), (1_700, 2_000)],
        3_700,
    );
    for beat in &mut plan.beats {
        beat.transition = Transition {
            kind: "crossfade".into(),
            duration_ms: 400,
        };
    }

    let rendered = renderer(&dir.join("cache"))
        .render(&plan, &mut |_| {})
        .expect("the fallback renders");
    assert_eq!(
        rendered.reused_ms, None,
        "a one-pass render reused nothing, and says so"
    );
    assert!((duration_of(&rendered.path) - 3.7).abs() < 0.15);
}

/// A cache with a size cap has to know which entries are still wanted. The
/// only thing that distinguishes a segment three builds have leaned on
/// from one nothing has asked for since April is that the renderer said so
/// when it copied from it.
#[test]
fn copying_from_a_segment_marks_it_as_used() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-used");
    let clip = bright_clip(&dir, "clip.mp4");
    let cache = dir.join("cache");
    let plan = plan(&dir, &clip, &[(0, 1_000), (2_000, 1_000)], 4_000);

    let renderer = renderer(&cache);
    renderer.render(&plan, &mut |_| {}).expect("renders cold");
    let segment = cache_state(&cache).remove(0);

    // Backdated, as an entry from an earlier session would be.
    let long_ago = SystemTime::now() - std::time::Duration::from_secs(86_400);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&segment)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(long_ago))
        .unwrap();

    renderer.render(&plan, &mut |_| {}).expect("renders warm");

    assert!(
        last_used(&segment) > long_ago,
        "a segment this render copied from still looks a day stale, so a \
         prune would throw away exactly what is being used"
    );
}
