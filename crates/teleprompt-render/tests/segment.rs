//! The output cut into independently encodable segments.
//!
//! This is the whole of the incremental render: if the segments tile the
//! timeline exactly, concatenating them is the same video; if a segment's
//! key covers everything that changes its bytes, a re-render only pays for
//! what moved. Both are arithmetic, so both are tested without ffmpeg.

use std::path::{Path, PathBuf};

use teleprompt_core::Hash;
use teleprompt_render::segment::{self, Content, Segment, SegmentKey, Source};
use teleprompt_render::{Beat, Picture, RenderPlan, Transition};

/// A resolver that gives every clip the same identity, for the tests that
/// are not about clip contents.
fn same(_: &Path) -> std::io::Result<Hash> {
    Ok(Hash::of(b"a clip"))
}

fn beat(id: &str, start_ms: u64, duration_ms: u64, clip: &str) -> Beat {
    Beat {
        id: id.into(),
        start_ms,
        duration_ms,
        picture: Picture::Clip(PathBuf::from(clip)),
        transition: Transition::cut(),
    }
}

fn plan(beats: Vec<Beat>, duration_ms: u64) -> RenderPlan {
    RenderPlan {
        width: 640,
        height: 360,
        fps: 25,
        duration_ms,
        beats,
        narration: Vec::new(),
        output: PathBuf::from("/out/tour.mp4"),
    }
}

/// The property everything else rests on. A segment list that does not
/// tile the timeline produces a video of the wrong length, and it does it
/// silently — the frames are all there, just not all of them.
#[track_caller]
fn tiles(segments: &[Segment], plan: &RenderPlan) {
    let mut frame = 0u64;
    for segment in segments {
        assert_eq!(
            segment.start_frame, frame,
            "a gap or an overlap between segments: {segments:#?}"
        );
        frame += segment.frames;
    }
    let expected = (plan.duration_ms * u64::from(plan.fps) + 500) / 1000;
    assert_eq!(
        frame, expected,
        "{frame} frame(s) for a {}ms plan at {}fps",
        plan.duration_ms, plan.fps,
    );
}

#[test]
fn a_plan_of_hard_cuts_is_one_segment_per_beat() {
    let plan = plan(
        vec![
            beat("a#0", 0, 2_000, "/clips/a.mp4"),
            beat("b#0", 2_000, 2_000, "/clips/b.mp4"),
        ],
        4_000,
    );
    let segments = segment::segments(&plan).expect("a plan of cuts splits");

    assert_eq!(segments.len(), 2);
    assert!(segments
        .iter()
        .all(|s| matches!(s.content, Content::Body(_))));
    tiles(&segments, &plan);
}

/// A gap between beats is held on the previous beat's last frame, which
/// makes it part of that beat's segment rather than a segment of its own.
/// Two renders of the same beat with a different amount of silence after it
/// are genuinely different pictures, and the key has to say so.
#[test]
fn a_held_gap_belongs_to_the_beat_that_holds_it() {
    let short = plan(vec![beat("a#0", 0, 1_000, "/clips/a.mp4")], 2_000);
    let long = plan(vec![beat("a#0", 0, 1_000, "/clips/a.mp4")], 5_000);

    let a = segment::segments(&short).expect("splits");
    let b = segment::segments(&long).expect("splits");

    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert!(b[0].frames > a[0].frames, "the held tail is in the segment");
    assert_ne!(
        SegmentKey::for_segment(&short, &a[0])
            .hash(&mut same)
            .unwrap(),
        SegmentKey::for_segment(&long, &b[0])
            .hash(&mut same)
            .unwrap(),
        "holding a frame for four seconds is not the same picture as \
         holding it for one"
    );
    tiles(&a, &short);
    tiles(&b, &long);
}

/// A transition is the one join that two beats share, so it cannot live in
/// either of them. It is a segment: body, blend, body.
#[test]
fn a_transition_becomes_a_segment_of_its_own() {
    let mut beats = vec![
        beat("a#0", 0, 2_000, "/clips/a.mp4"),
        // 400ms of overlap: the scheduler starts the next beat before this
        // one ends, which is how the manifest expresses a transition.
        beat("b#0", 1_600, 2_000, "/clips/b.mp4"),
    ];
    beats[0].transition = Transition {
        kind: "dissolve".into(),
        duration_ms: 400,
    };
    let plan = plan(beats, 3_600);
    let segments = segment::segments(&plan).expect("splits");

    assert_eq!(segments.len(), 3, "body, blend, body: {segments:#?}");
    let Content::Blend { kind, from, to } = &segments[1].content else {
        panic!("the middle segment is the blend: {segments:#?}");
    };
    assert_eq!(kind, "dissolve");
    assert_eq!(segments[1].frames, 10, "400ms at 25fps");
    assert_eq!(from.frames, 10);
    assert_eq!(to.frames, 10);
    assert_eq!(
        to.from_frame, 0,
        "the blend takes the head of the next beat"
    );
    assert_eq!(
        from.source,
        Source::Clip(PathBuf::from("/clips/a.mp4")),
        "and the tail of the one it leaves"
    );
    tiles(&segments, &plan);
}

/// Two beats that share a blend must not also each render the frames the
/// blend consumed, or the video is longer than the plan and everything
/// after the first transition is late.
#[test]
fn the_beats_either_side_of_a_blend_give_up_the_frames_it_uses() {
    let mut beats = vec![
        beat("a#0", 0, 2_000, "/clips/a.mp4"),
        beat("b#0", 1_600, 2_000, "/clips/b.mp4"),
    ];
    beats[0].transition = Transition {
        kind: "crossfade".into(),
        duration_ms: 400,
    };
    let plan = plan(beats, 3_600);
    let segments = segment::segments(&plan).expect("splits");

    assert_eq!(segments[0].frames, 40, "2000ms less the 400ms blend");
    assert_eq!(segments[2].frames, 40, "and the same at the other end");
    tiles(&segments, &plan);
}

/// Nothing captured at all is still a render, and still a segment.
#[test]
fn an_uncaptured_plan_is_a_slate_segment() {
    let mut beats = vec![beat("a#0", 0, 2_000, "/clips/a.mp4")];
    beats[0].picture = Picture::Slate;
    let plan = plan(beats, 2_000);
    let segments = segment::segments(&plan).expect("splits");

    let Content::Body(window) = &segments[0].content else {
        panic!("a slate is a body");
    };
    assert_eq!(window.source, Source::Slate);
    tiles(&segments, &plan);
}

/// The reason the cache exists: the same picture in the same place is the
/// same bytes, whichever script asked for it.
#[test]
fn two_identical_segments_share_a_key() {
    let one = plan(vec![beat("a#0", 0, 2_000, "/clips/a.mp4")], 2_000);
    let two = plan(vec![beat("elsewhere#7", 0, 2_000, "/clips/a.mp4")], 2_000);

    let a = segment::segments(&one).expect("splits");
    let b = segment::segments(&two).expect("splits");
    assert_eq!(
        SegmentKey::for_segment(&one, &a[0])
            .hash(&mut same)
            .unwrap(),
        SegmentKey::for_segment(&two, &b[0])
            .hash(&mut same)
            .unwrap(),
        "a beat's name is not part of its picture"
    );
}

/// The failure mode of every input-addressed key is the input somebody
/// forgot: it does not error, it serves the wrong frames. Each of these is
/// a thing that changes what the encoder emits.
#[test]
fn everything_that_changes_the_bytes_changes_the_key() {
    let base = plan(vec![beat("a#0", 0, 2_000, "/clips/a.mp4")], 2_000);
    let segments = segment::segments(&base).expect("splits");
    let key_of = |plan: &RenderPlan, resolve: &mut dyn FnMut(&Path) -> std::io::Result<Hash>| {
        let segments = segment::segments(plan).expect("splits");
        SegmentKey::for_segment(plan, &segments[0])
            .hash(resolve)
            .unwrap()
    };
    let original = SegmentKey::for_segment(&base, &segments[0])
        .hash(&mut same)
        .unwrap();

    let mut wider = base.clone();
    wider.width = 1280;
    assert_ne!(original, key_of(&wider, &mut same), "frame size");

    let mut taller = base.clone();
    taller.height = 720;
    assert_ne!(original, key_of(&taller, &mut same), "frame height");

    let mut faster = base.clone();
    faster.fps = 30;
    assert_ne!(original, key_of(&faster, &mut same), "frame rate");

    let mut longer = base.clone();
    longer.beats[0].duration_ms = 3_000;
    longer.duration_ms = 3_000;
    assert_ne!(original, key_of(&longer, &mut same), "beat length");

    assert_ne!(
        original,
        key_of(&base, &mut |_| Ok(Hash::of(b"a re-captured clip"))),
        "the clip's own contents — a re-capture at the same path is a \
         different picture, and a key on the path alone would serve the old \
         one for ever"
    );
}

/// A clip that cannot be read is a render that cannot be keyed. Guessing a
/// key for it would cache the failure.
#[test]
fn a_clip_that_cannot_be_read_fails_the_key_rather_than_guessing_one() {
    let base = plan(vec![beat("a#0", 0, 2_000, "/clips/gone.mp4")], 2_000);
    let segments = segment::segments(&base).expect("splits");
    let result = SegmentKey::for_segment(&base, &segments[0]).hash(&mut |path| {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    });
    assert!(result.is_err());
}

/// Blends that eat more of a beat than the beat has cannot be cut into
/// independent segments — the two joins would draw the same frames twice.
/// Reporting that honestly is what lets the caller fall back to rendering
/// the whole graph in one pass.
#[test]
fn a_beat_shorter_than_the_blends_around_it_does_not_split() {
    let mut beats = vec![
        beat("a#0", 0, 2_000, "/clips/a.mp4"),
        beat("b#0", 1_600, 500, "/clips/b.mp4"),
        beat("c#0", 1_700, 2_000, "/clips/c.mp4"),
    ];
    for b in &mut beats {
        b.transition = Transition {
            kind: "crossfade".into(),
            duration_ms: 400,
        };
    }
    let plan = plan(beats, 3_700);
    assert!(
        segment::segments(&plan).is_none(),
        "400ms in and 400ms out of a 500ms beat overlap"
    );
}

/// The property that makes a rebuild cheap, and the one that is easy to
/// lose. Rewording a sentence at the top of a script moves everything
/// after it, usually by some fraction of a frame — and a segment whose
/// length was read off the global timeline rounds the other way when that
/// happens. A beat that did not otherwise change has to keep its key
/// through a shift, or a one-word edit re-encodes half a video that nobody
/// touched.
#[test]
fn a_beat_that_only_slid_along_the_timeline_keeps_its_key() {
    // 1030ms at 25fps is 25.75 frames — not a whole number, which is what
    // makes it sensitive to where on the timeline it lands.
    // The beat under test is deliberately not the last one: the last
    // segment is where the rounding of the whole video is settled, so it
    // is the one segment that does depend on everything before it.
    let before = plan(
        vec![
            beat("a#0", 0, 1_000, "/clips/a.mp4"),
            beat("b#0", 1_000, 1_030, "/clips/b.mp4"),
            beat("c#0", 2_030, 1_000, "/clips/c.mp4"),
        ],
        3_030,
    );
    // The sentence over the first beat got twenty milliseconds longer.
    let after = plan(
        vec![
            beat("a#0", 0, 1_020, "/clips/a.mp4"),
            beat("b#0", 1_020, 1_030, "/clips/b.mp4"),
            beat("c#0", 2_050, 1_000, "/clips/c.mp4"),
        ],
        3_050,
    );

    let (a, b) = (
        segment::segments(&before).expect("splits"),
        segment::segments(&after).expect("splits"),
    );
    tiles(&a, &before);
    tiles(&b, &after);
    assert_eq!(
        SegmentKey::for_segment(&before, &a[1])
            .hash(&mut same)
            .unwrap(),
        SegmentKey::for_segment(&after, &b[1])
            .hash(&mut same)
            .unwrap(),
        "the second beat shows the same clip for the same length; only \
         everything before it moved"
    );
}
