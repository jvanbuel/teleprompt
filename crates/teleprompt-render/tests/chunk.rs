//! The output cut into independently encodable chunks.
//!
//! This is the whole of the incremental render: if the chunks tile the
//! timeline exactly, concatenating them is the same video; if a chunk's
//! key covers everything that changes its bytes, a re-render only pays for
//! what moved. Both are arithmetic, so both are tested without ffmpeg.

use std::path::{Path, PathBuf};

use teleprompt_core::Hash;
use teleprompt_render::chunk::{self, Chunk, ChunkKey, Content, Source};
use teleprompt_render::{Cue, Picture, RenderPlan, Transition};

/// A resolver that gives every clip the same identity, for the tests that
/// are not about clip contents.
fn same(_: &Path) -> std::io::Result<Hash> {
    Ok(Hash::of(b"a clip"))
}

fn cue(id: &str, start_ms: u64, duration_ms: u64, clip: &str) -> Cue {
    Cue {
        id: id.into(),
        start_ms,
        duration_ms,
        picture: Picture::Clip(PathBuf::from(clip)),
        transition: Transition::cut(),
    }
}

fn plan(cues: Vec<Cue>, duration_ms: u64) -> RenderPlan {
    RenderPlan {
        width: 640,
        height: 360,
        fps: 25,
        duration_ms,
        cues,
        narration: Vec::new(),
        output: PathBuf::from("/out/tour.mp4"),
    }
}

/// The property everything else rests on. A chunk list that does not
/// tile the timeline produces a video of the wrong length, and it does it
/// silently — the frames are all there, just not all of them.
#[track_caller]
fn tiles(chunks: &[Chunk], plan: &RenderPlan) {
    let mut frame = 0u64;
    for chunk in chunks {
        assert_eq!(
            chunk.start_frame, frame,
            "a gap or an overlap between chunks: {chunks:#?}"
        );
        frame += chunk.frames;
    }
    let expected = (plan.duration_ms * u64::from(plan.fps) + 500) / 1000;
    assert_eq!(
        frame, expected,
        "{frame} frame(s) for a {}ms plan at {}fps",
        plan.duration_ms, plan.fps,
    );
}

#[test]
fn a_plan_of_hard_cuts_is_one_chunk_per_beat() {
    let plan = plan(
        vec![
            cue("a#0", 0, 2_000, "/clips/a.mp4"),
            cue("b#0", 2_000, 2_000, "/clips/b.mp4"),
        ],
        4_000,
    );
    let chunks = chunk::chunks(&plan).expect("a plan of cuts splits");

    assert_eq!(chunks.len(), 2);
    assert!(chunks.iter().all(|s| matches!(s.content, Content::Body(_))));
    tiles(&chunks, &plan);
}

/// A gap between cues is held on the previous cue's last frame, which
/// makes it part of that cue's chunk rather than a chunk of its own.
/// Two renders of the same cue with a different amount of silence after it
/// are genuinely different pictures, and the key has to say so.
#[test]
fn a_held_gap_belongs_to_the_beat_that_holds_it() {
    let short = plan(vec![cue("a#0", 0, 1_000, "/clips/a.mp4")], 2_000);
    let long = plan(vec![cue("a#0", 0, 1_000, "/clips/a.mp4")], 5_000);

    let a = chunk::chunks(&short).expect("splits");
    let b = chunk::chunks(&long).expect("splits");

    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert!(b[0].frames > a[0].frames, "the held tail is in the chunk");
    assert_ne!(
        ChunkKey::for_chunk(&short, &a[0]).hash(&mut same).unwrap(),
        ChunkKey::for_chunk(&long, &b[0]).hash(&mut same).unwrap(),
        "holding a frame for four seconds is not the same picture as \
         holding it for one"
    );
    tiles(&a, &short);
    tiles(&b, &long);
}

/// A transition is the one join that two cues share, so it cannot live in
/// either of them. It is a chunk: body, blend, body.
#[test]
fn a_transition_becomes_a_chunk_of_its_own() {
    let mut cues = vec![
        cue("a#0", 0, 2_000, "/clips/a.mp4"),
        // 400ms of overlap: the scheduler starts the next cue before this
        // one ends, which is how the manifest expresses a transition.
        cue("b#0", 1_600, 2_000, "/clips/b.mp4"),
    ];
    cues[0].transition = Transition {
        kind: "dissolve".into(),
        duration_ms: 400,
    };
    let plan = plan(cues, 3_600);
    let chunks = chunk::chunks(&plan).expect("splits");

    assert_eq!(chunks.len(), 3, "body, blend, body: {chunks:#?}");
    let Content::Blend { kind, from, to } = &chunks[1].content else {
        panic!("the middle chunk is the blend: {chunks:#?}");
    };
    assert_eq!(kind, "dissolve");
    assert_eq!(chunks[1].frames, 10, "400ms at 25fps");
    assert_eq!(from.frames, 10);
    assert_eq!(to.frames, 10);
    assert_eq!(to.from_frame, 0, "the blend takes the head of the next cue");
    assert_eq!(
        from.source,
        Source::Clip(PathBuf::from("/clips/a.mp4")),
        "and the tail of the one it leaves"
    );
    tiles(&chunks, &plan);
}

/// Two cues that share a blend must not also each render the frames the
/// blend consumed, or the video is longer than the plan and everything
/// after the first transition is late.
#[test]
fn the_beats_either_side_of_a_blend_give_up_the_frames_it_uses() {
    let mut cues = vec![
        cue("a#0", 0, 2_000, "/clips/a.mp4"),
        cue("b#0", 1_600, 2_000, "/clips/b.mp4"),
    ];
    cues[0].transition = Transition {
        kind: "crossfade".into(),
        duration_ms: 400,
    };
    let plan = plan(cues, 3_600);
    let chunks = chunk::chunks(&plan).expect("splits");

    assert_eq!(chunks[0].frames, 40, "2000ms less the 400ms blend");
    assert_eq!(chunks[2].frames, 40, "and the same at the other end");
    tiles(&chunks, &plan);
}

/// Nothing captured at all is still a render, and still a chunk.
#[test]
fn an_uncaptured_plan_is_a_slate_chunk() {
    let mut cues = vec![cue("a#0", 0, 2_000, "/clips/a.mp4")];
    cues[0].picture = Picture::Slate;
    let plan = plan(cues, 2_000);
    let chunks = chunk::chunks(&plan).expect("splits");

    let Content::Body(window) = &chunks[0].content else {
        panic!("a slate is a body");
    };
    assert_eq!(window.source, Source::Slate);
    tiles(&chunks, &plan);
}

/// The reason the cache exists: the same picture in the same place is the
/// same bytes, whichever script asked for it.
#[test]
fn two_identical_chunks_share_a_key() {
    let one = plan(vec![cue("a#0", 0, 2_000, "/clips/a.mp4")], 2_000);
    let two = plan(vec![cue("elsewhere#7", 0, 2_000, "/clips/a.mp4")], 2_000);

    let a = chunk::chunks(&one).expect("splits");
    let b = chunk::chunks(&two).expect("splits");
    assert_eq!(
        ChunkKey::for_chunk(&one, &a[0]).hash(&mut same).unwrap(),
        ChunkKey::for_chunk(&two, &b[0]).hash(&mut same).unwrap(),
        "a cue's name is not part of its picture"
    );
}

/// The failure mode of every input-addressed key is the input somebody
/// forgot: it does not error, it serves the wrong frames. Each of these is
/// a thing that changes what the encoder emits.
#[test]
fn everything_that_changes_the_bytes_changes_the_key() {
    let base = plan(vec![cue("a#0", 0, 2_000, "/clips/a.mp4")], 2_000);
    let chunks = chunk::chunks(&base).expect("splits");
    let key_of = |plan: &RenderPlan, resolve: &mut dyn FnMut(&Path) -> std::io::Result<Hash>| {
        let chunks = chunk::chunks(plan).expect("splits");
        ChunkKey::for_chunk(plan, &chunks[0]).hash(resolve).unwrap()
    };
    let original = ChunkKey::for_chunk(&base, &chunks[0])
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
    longer.cues[0].duration_ms = 3_000;
    longer.duration_ms = 3_000;
    assert_ne!(original, key_of(&longer, &mut same), "cue length");

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
    let base = plan(vec![cue("a#0", 0, 2_000, "/clips/gone.mp4")], 2_000);
    let chunks = chunk::chunks(&base).expect("splits");
    let result = ChunkKey::for_chunk(&base, &chunks[0]).hash(&mut |path| {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    });
    assert!(result.is_err());
}

/// Blends that eat more of a cue than the cue has cannot be cut into
/// independent chunks — the two joins would draw the same frames twice.
/// Reporting that honestly is what lets the caller fall back to rendering
/// the whole graph in one pass.
#[test]
fn a_beat_shorter_than_the_blends_around_it_does_not_split() {
    let mut cues = vec![
        cue("a#0", 0, 2_000, "/clips/a.mp4"),
        cue("b#0", 1_600, 500, "/clips/b.mp4"),
        cue("c#0", 1_700, 2_000, "/clips/c.mp4"),
    ];
    for b in &mut cues {
        b.transition = Transition {
            kind: "crossfade".into(),
            duration_ms: 400,
        };
    }
    let plan = plan(cues, 3_700);
    assert!(
        chunk::chunks(&plan).is_none(),
        "400ms in and 400ms out of a 500ms cue overlap"
    );
}

/// The property that makes a rebuild cheap, and the one that is easy to
/// lose. Rewording a sentence at the top of a script moves everything
/// after it, usually by some fraction of a frame — and a chunk whose
/// length was read off the global timeline rounds the other way when that
/// happens. A cue that did not otherwise change has to keep its key
/// through a shift, or a one-word edit re-encodes half a video that nobody
/// touched.
#[test]
fn a_cue_that_only_slid_along_the_timeline_keeps_its_key() {
    // 1030ms at 25fps is 25.75 frames — not a whole number, which is what
    // makes it sensitive to where on the timeline it lands.
    // The cue under test is deliberately not the last one: the last
    // chunk is where the rounding of the whole video is settled, so it
    // is the one chunk that does depend on everything before it.
    let before = plan(
        vec![
            cue("a#0", 0, 1_000, "/clips/a.mp4"),
            cue("b#0", 1_000, 1_030, "/clips/b.mp4"),
            cue("c#0", 2_030, 1_000, "/clips/c.mp4"),
        ],
        3_030,
    );
    // The sentence over the first cue got twenty milliseconds longer.
    let after = plan(
        vec![
            cue("a#0", 0, 1_020, "/clips/a.mp4"),
            cue("b#0", 1_020, 1_030, "/clips/b.mp4"),
            cue("c#0", 2_050, 1_000, "/clips/c.mp4"),
        ],
        3_050,
    );

    let (a, b) = (
        chunk::chunks(&before).expect("splits"),
        chunk::chunks(&after).expect("splits"),
    );
    tiles(&a, &before);
    tiles(&b, &after);
    assert_eq!(
        ChunkKey::for_chunk(&before, &a[1]).hash(&mut same).unwrap(),
        ChunkKey::for_chunk(&after, &b[1]).hash(&mut same).unwrap(),
        "the second cue shows the same clip for the same length; only \
         everything before it moved"
    );
}
