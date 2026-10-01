//! The incremental renderer against real ffmpeg.
//!
//! Two claims to hold down. The first is that cutting a video into cached
//! chunks and copying them back together produces the same video — same
//! length, same picture, narration in the same place. The second is that
//! the cache actually skips work: a rebuild after a small edit has to
//! re-encode the part that moved and nothing else, and the only honest way
//! to check that is to look at which files the renderer touched.

use std::path::{Path, PathBuf};
use std::time::SystemTime;
use teleprompt_core::config::TransitionKind;
use teleprompt_core::{SpanMs, TimeMs};

use teleprompt_render::incremental::IncrementalRenderer;
use teleprompt_render::{Narration, Picture, RenderPlan, Shot, Transition};

mod support;
use support::{bright_clip, duration_of, first_sound, have_ffmpeg, luma_at, workdir};

fn renderer(cache: &Path) -> IncrementalRenderer {
    IncrementalRenderer {
        program: "ffmpeg".into(),
        cache_dir: cache.to_path_buf(),
        reuse: true,
    }
}

/// Every cached chunk.
fn cache_state(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("the cache directory exists")
        .map(|e| e.expect("a readable entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "mp4"))
        .collect();
    entries.sort();
    entries
}

/// When a cached chunk was last used.
fn last_used(path: &Path) -> SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

fn plan(dir: &Path, clip: &Path, shots: &[(u64, u64)], duration_ms: u64) -> RenderPlan {
    RenderPlan {
        width: 320,
        height: 180,
        fps: 24,
        duration_ms: SpanMs::of(duration_ms),
        shots: shots
            .iter()
            .enumerate()
            .map(|(i, (start_ms, duration_ms))| Shot {
                id: format!("b{i}#0").into(),
                start_ms: TimeMs::at(*start_ms),
                duration_ms: SpanMs::of(*duration_ms),
                picture: Picture::Clip(clip.to_path_buf()),
                transition: Transition::cut(),
            })
            .collect(),
        narration: Vec::new(),
        titles: Vec::new(),
        output: dir.join("out.mp4"),
    }
}

/// The renderer with reuse switched off — what `--no-cache` asks for, and
/// what these tests want: every frame encoded here, nothing served from a
/// cache a previous test filled.
///
/// Since the second renderer was deleted this is the *same* renderer with
/// its cache ignored, which is a stronger comparison than before: the two
/// paths can no longer disagree about what the video should look like,
/// only about how much of it had to be encoded.
fn one_pass(dir: &Path) -> IncrementalRenderer {
    IncrementalRenderer {
        program: "ffmpeg".into(),
        // Inside the test's own directory. These run in parallel in one
        // process, and a chunk key is content-addressed, so two tests
        // rendering similar plans into a shared cache write the *same*
        // filename — which is a half-written mp4 and `moov atom not found`.
        cache_dir: dir.join("chunks"),
        reuse: false,
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
    one_pass(&dir)
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
        "one pass rendered {a}s and chunks rendered {b}s"
    );
    assert!((b - 5.0).abs() < 0.1, "a 5s plan rendered {b}s");
    assert_eq!(
        rendered.reused_ms,
        Some(0),
        "nothing was cached before this render"
    );
}

/// A gap between shots is held on the last frame, and it is held through
/// the concat too — the seam between two chunks must not be a flash of
/// black.
#[test]
fn the_picture_holds_across_the_seam_between_chunks() {
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
        start_ms: TimeMs::at(2_000),
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

/// The file a person plays carries an audio track a player is expecting.
///
/// This is not the same claim as "the mix has sound in it", which
/// `narration_lands_where_it_was_placed_through_the_concat` already makes,
/// and it is the one that was wrong: the mix ran at the voice backend's
/// own 24 kHz mono and the encode shipped it out unchanged, so every
/// render carried a legal, small, and unusual audio track. The symptom is
/// not an error anywhere — the track is present, the container declares
/// it, `volumedetect` reads it back at a healthy level, and the person
/// watching hears nothing.
#[test]
fn the_rendered_file_carries_a_track_a_player_expects() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-delivery");
    let clip = bright_clip(&dir, "clip.mp4");
    let mut plan = plan(&dir, &clip, &[(0, 1_000), (1_000, 1_000)], 2_000);
    plan.narration = vec![Narration {
        id: "line".into(),
        path: support::tone(&dir, "line.wav", 1_000),
        start_ms: TimeMs::at(0),
    }];

    renderer(&dir.join("cache"))
        .render(&plan, &mut |_| {})
        .expect("renders");

    assert_eq!(
        support::audio_track(&plan.output),
        (48_000, 2),
        "the bed mixes at 24 kHz mono; what comes out is for playing"
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
        "a warm render added or dropped a chunk"
    );
    assert_eq!(
        rendered.reused_ms,
        Some(4_000),
        "every frame came from the cache"
    );
    assert!((duration_of(&rendered.path) - 4.0).abs() < 0.1);
}

/// And the case that makes it worth having: one shot moves, and the rest
/// of the video is copied rather than encoded. This is what a rebuild after
/// an edit looks like.
#[test]
fn changing_one_shot_re_encodes_only_the_chunks_that_moved() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-edit");
    // Three different pictures, because two shots showing the same frames
    // for the same length *are* the same chunk — the cache is entitled
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
    plan.shots[1].picture = Picture::Clip(b);
    plan.shots[2].picture = Picture::Clip(c);

    renderer.render(&plan, &mut |_| {}).expect("renders cold");
    let before = cache_state(&cache);
    assert_eq!(before.len(), 3, "one chunk per shot: {before:#?}");

    // The middle shot now shows something else. The shots either side of
    // it did not move — same clip, same slot, same held tail — so their
    // chunks are still exactly the frames already on disk.
    plan.shots[1].picture = Picture::Clip(edited);
    let rendered = renderer
        .render(&plan, &mut |_| {})
        .expect("renders after the edit");

    assert_eq!(
        rendered.reused_ms,
        Some(4_000),
        "the two untouched shots — 2s each, held gaps included — came \
         from the cache"
    );
    let after = cache_state(&cache);
    assert_eq!(after.len(), 4, "one chunk encoded, three kept: {after:#?}");
    assert!(
        before.iter().all(|e| after.contains(e)),
        "a chunk that did not change was dropped:\n{before:#?}\n{after:#?}"
    );
    assert!((duration_of(&rendered.path) - 6.0).abs() < 0.1);
}

/// A transition is a chunk of its own, so a plan that has one still has
/// to come out the length the scheduler planned for it.
#[test]
fn a_crossfade_survives_being_a_chunk_of_its_own() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-xfade");
    let clip = bright_clip(&dir, "clip.mp4");
    let mut plan = plan(&dir, &clip, &[(0, 2_000), (1_500, 2_000)], 3_500);
    plan.shots[0].transition = Transition {
        kind: TransitionKind::Crossfade,
        duration_ms: SpanMs::of(500),
    };

    let rendered = renderer(&dir.join("cache"))
        .render(&plan, &mut |_| {})
        .expect("renders");

    let seconds = duration_of(&rendered.path);
    assert!(
        (seconds - 3.5).abs() < 0.1,
        "two 2s shots crossfaded by 0.5s make 3.5s of picture, not {seconds}s"
    );
    for t in [0.5, 1.6, 3.0] {
        let luma = luma_at(&rendered.path, t);
        assert!(luma > 60.0, "the picture went dark {t}s in: YAVG {luma}");
    }
}

/// A plan that cannot be cut is reported, not rerouted.
///
/// This used to render in one pass through a second renderer — slower,
/// and silently: the report still named the incremental path while the
/// other one did the work. A fallback nobody can see is how two
/// implementations drift apart, which this project has now paid for more
/// than once.
///
/// The shape is unreachable from a script. A shot shorter than the
/// transitions either side of it means the two blends would draw the same
/// frames twice, and the scheduler caps a transition against what the item
/// it arrives in has left, so it cannot arise (see
/// `a_transition_leaves_room_for_the_one_that_arrived_before_it`). Only a
/// hand-built plan can be this shape, and the right answer for one is to
/// say so.
#[test]
fn a_plan_that_cannot_be_cut_is_reported_rather_than_rerouted() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-unchunkable");
    let clip = bright_clip(&dir, "clip.mp4");
    // A 500ms shot with 400ms of blend at each end.
    let mut plan = plan(
        &dir,
        &clip,
        &[(0, 2_000), (1_600, 500), (1_700, 2_000)],
        3_700,
    );
    for shot in &mut plan.shots {
        shot.transition = Transition {
            kind: TransitionKind::Crossfade,
            duration_ms: SpanMs::of(400),
        };
    }

    let why = renderer(&dir.join("cache"))
        .render(&plan, &mut |_| {})
        .expect_err("a plan that cannot be cut is an error, not a detour");
    let said = why.to_string();
    assert!(
        said.contains("shorter than the transitions"),
        "the error should say what is wrong with the plan: {said}"
    );
}

/// A cache with a size cap has to know which entries are still wanted. The
/// only thing that distinguishes a chunk three builds have leaned on
/// from one nothing has asked for since April is that the renderer said so
/// when it copied from it.
#[test]
fn copying_from_a_chunk_marks_it_as_used() {
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
    let chunk = cache_state(&cache).remove(0);

    // Backdated, as an entry from an earlier session would be.
    let long_ago = SystemTime::now() - std::time::Duration::from_secs(86_400);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&chunk)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(long_ago))
        .unwrap();

    renderer.render(&plan, &mut |_| {}).expect("renders warm");

    assert!(
        last_used(&chunk) > long_ago,
        "a chunk this render copied from still looks a day stale, so a \
         prune would throw away exactly what is being used"
    );
}

/// A final pass that dies part way (Ctrl+C, a full disk) leaves the last
/// good video at `--out`, not the half of a new one.
#[cfg(unix)]
#[test]
fn a_render_that_fails_at_the_end_leaves_the_output_as_it_was() {
    use std::os::unix::fs::PermissionsExt;
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("incr-atomic");
    let clip = bright_clip(&dir, "clip.mp4");
    let plan = plan(&dir, &clip, &[(0, 1_000)], 1_000);
    std::fs::write(&plan.output, b"the last good render").unwrap();
    // Real ffmpeg for the chunks; the concat pass writes half a file to
    // the path it was given, and fails.
    let wrapper = dir.join("ffmpeg-dies-at-the-end");
    std::fs::write(
        &wrapper,
        "#!/bin/sh\ncase \"$*\" in *concat*) for last; do :; done; \
         printf half > \"$last\"; exit 1;; esac\nexec ffmpeg \"$@\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();

    let renderer = IncrementalRenderer {
        program: wrapper.display().to_string(),
        ..one_pass(&dir)
    };
    renderer
        .render(&plan, &mut |_| {})
        .expect_err("the final pass failed");
    assert_eq!(
        std::fs::read(&plan.output).unwrap(),
        b"the last good render"
    );
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("partial"))
        .collect();
    assert!(left.is_empty(), "a partial file was left: {left:?}");
}
