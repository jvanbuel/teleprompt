//! The output cut into independently encodable chunks.
//!
//! This is the whole of the incremental render: if the chunks tile the
//! timeline exactly, concatenating them is the same video; if a chunk's
//! key covers everything that changes its bytes, a re-render only pays for
//! what moved. Both are arithmetic, so both are tested without ffmpeg.

use std::path::{Path, PathBuf};
use teleprompt_core::config::TransitionKind;

use teleprompt_core::{Hash, SpanMs, TimeMs};
use teleprompt_render::chunk::{self, Chunk, ChunkKey, Content, Source};
use teleprompt_render::{Picture, RenderPlan, Shot, Transition};

/// A resolver that gives every clip the same identity, for the tests that
/// are not about clip contents.
fn same(_: &Path) -> std::io::Result<Hash> {
    Ok(Hash::of(b"a clip"))
}

fn shot(id: &str, start_ms: u64, duration_ms: u64, clip: &str) -> Shot {
    Shot {
        id: id.into(),
        start_ms: TimeMs::at(start_ms),
        duration_ms: SpanMs::of(duration_ms),
        picture: Picture::Clip(PathBuf::from(clip)),
        transition: Transition::cut(),
    }
}

/// A shot that hands over with a crossfade rather than a cut.
fn blended(id: &str, start_ms: u64, duration_ms: u64, clip: &str) -> Shot {
    Shot {
        transition: Transition {
            kind: TransitionKind::parse("xfade"),
            duration_ms: SpanMs::of(400),
        },
        ..shot(id, start_ms, duration_ms, clip)
    }
}

fn plan(shots: Vec<Shot>, duration_ms: u64) -> RenderPlan {
    RenderPlan {
        width: 640,
        height: 360,
        fps: 25,
        duration_ms: SpanMs::of(duration_ms),
        shots,
        narration: Vec::new(),
        titles: Vec::new(),
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
    let expected = (plan.duration_ms.ms() * u64::from(plan.fps) + 500) / 1000;
    assert_eq!(
        frame,
        expected,
        "{frame} frame(s) for a {}ms plan at {}fps",
        plan.duration_ms.ms(),
        plan.fps,
    );
}

#[test]
fn a_plan_of_hard_cuts_is_one_chunk_per_shot() {
    let plan = plan(
        vec![
            shot("a#0", 0, 2_000, "/clips/a.mp4"),
            shot("b#0", 2_000, 2_000, "/clips/b.mp4"),
        ],
        4_000,
    );
    let chunks = chunk::chunks(&plan).expect("a plan of cuts splits");

    assert_eq!(chunks.len(), 2);
    assert!(chunks.iter().all(|s| matches!(s.content, Content::Body(_))));
    tiles(&chunks, &plan);
}

/// A gap between shots is held on the previous shot's last frame, which
/// makes it part of that shot's chunk rather than a chunk of its own.
/// Two renders of the same shot with a different amount of silence after it
/// are genuinely different pictures, and the key has to say so.
#[test]
fn a_held_gap_belongs_to_the_shot_that_holds_it() {
    let short = plan(vec![shot("a#0", 0, 1_000, "/clips/a.mp4")], 2_000);
    let long = plan(vec![shot("a#0", 0, 1_000, "/clips/a.mp4")], 5_000);

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

/// A transition is the one join that two shots share, so it cannot live in
/// either of them. It is a chunk: body, blend, body.
#[test]
fn a_transition_becomes_a_chunk_of_its_own() {
    let mut shots = vec![
        shot("a#0", 0, 2_000, "/clips/a.mp4"),
        // 400ms of overlap: the scheduler starts the next shot before this
        // one ends, which is how the manifest expresses a transition.
        shot("b#0", 1_600, 2_000, "/clips/b.mp4"),
    ];
    shots[0].transition = Transition {
        kind: TransitionKind::Dissolve,
        duration_ms: SpanMs::of(400),
    };
    let plan = plan(shots, 3_600);
    let chunks = chunk::chunks(&plan).expect("splits");

    assert_eq!(chunks.len(), 3, "body, blend, body: {chunks:#?}");
    let Content::Blend { kind, from, to } = &chunks[1].content else {
        panic!("the middle chunk is the blend: {chunks:#?}");
    };
    assert_eq!(*kind, TransitionKind::Dissolve);
    assert_eq!(chunks[1].frames, 10, "400ms at 25fps");
    assert_eq!(from.frames, 10);
    assert_eq!(to.frames, 10);
    assert_eq!(
        to.from_frame, 0,
        "the blend takes the head of the next shot"
    );
    assert_eq!(
        from.source,
        Source::Clip(PathBuf::from("/clips/a.mp4")),
        "and the tail of the one it leaves"
    );
    tiles(&chunks, &plan);
}

/// Two shots that share a blend must not also each render the frames the
/// blend consumed, or the video is longer than the plan and everything
/// after the first transition is late.
#[test]
fn the_shots_either_side_of_a_blend_give_up_the_frames_it_uses() {
    let mut shots = vec![
        shot("a#0", 0, 2_000, "/clips/a.mp4"),
        shot("b#0", 1_600, 2_000, "/clips/b.mp4"),
    ];
    shots[0].transition = Transition {
        kind: TransitionKind::Crossfade,
        duration_ms: SpanMs::of(400),
    };
    let plan = plan(shots, 3_600);
    let chunks = chunk::chunks(&plan).expect("splits");

    assert_eq!(chunks[0].frames, 40, "2000ms less the 400ms blend");
    assert_eq!(chunks[2].frames, 40, "and the same at the other end");
    tiles(&chunks, &plan);
}

/// Nothing captured at all is still a render, and still a chunk.
#[test]
fn an_uncaptured_plan_is_a_slate_chunk() {
    let mut shots = vec![shot("a#0", 0, 2_000, "/clips/a.mp4")];
    shots[0].picture = Picture::Slate;
    let plan = plan(shots, 2_000);
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
    let one = plan(vec![shot("a#0", 0, 2_000, "/clips/a.mp4")], 2_000);
    let two = plan(vec![shot("elsewhere#7", 0, 2_000, "/clips/a.mp4")], 2_000);

    let a = chunk::chunks(&one).expect("splits");
    let b = chunk::chunks(&two).expect("splits");
    assert_eq!(
        ChunkKey::for_chunk(&one, &a[0]).hash(&mut same).unwrap(),
        ChunkKey::for_chunk(&two, &b[0]).hash(&mut same).unwrap(),
        "a shot's name is not part of its picture"
    );
}

/// The failure mode of every input-addressed key is the input somebody
/// forgot: it does not error, it serves the wrong frames. Each of these is
/// a thing that changes what the encoder emits.
#[test]
fn everything_that_changes_the_bytes_changes_the_key() {
    let base = plan(vec![shot("a#0", 0, 2_000, "/clips/a.mp4")], 2_000);
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
    longer.shots[0].duration_ms = SpanMs::of(3_000);
    longer.duration_ms = SpanMs::of(3_000);
    assert_ne!(original, key_of(&longer, &mut same), "shot length");

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
    let base = plan(vec![shot("a#0", 0, 2_000, "/clips/gone.mp4")], 2_000);
    let chunks = chunk::chunks(&base).expect("splits");
    let result = ChunkKey::for_chunk(&base, &chunks[0]).hash(&mut |path| {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    });
    assert!(result.is_err());
}

/// Blends that eat more of a shot than the shot has cannot be cut into
/// independent chunks — the two joins would draw the same frames twice.
/// Reporting that honestly is what lets the caller fall back to rendering
/// the whole graph in one pass.
#[test]
fn a_shot_shorter_than_the_blends_around_it_does_not_split() {
    let mut shots = vec![
        shot("a#0", 0, 2_000, "/clips/a.mp4"),
        shot("b#0", 1_600, 500, "/clips/b.mp4"),
        shot("c#0", 1_700, 2_000, "/clips/c.mp4"),
    ];
    for b in &mut shots {
        b.transition = Transition {
            kind: TransitionKind::Crossfade,
            duration_ms: SpanMs::of(400),
        };
    }
    let plan = plan(shots, 3_700);
    assert!(
        chunk::chunks(&plan).is_none(),
        "400ms in and 400ms out of a 500ms shot overlap"
    );
}

/// The property that makes a rebuild cheap, and the one that is easy to
/// lose. Rewording a sentence at the top of a script moves everything
/// after it, usually by some fraction of a frame — and a chunk whose
/// length was read off the global timeline rounds the other way when that
/// happens. A shot that did not otherwise change has to keep its key
/// through a shift, or a one-word edit re-encodes half a video that nobody
/// touched.
#[test]
fn a_cue_that_only_slid_along_the_timeline_keeps_its_key() {
    // 1030ms at 25fps is 25.75 frames — not a whole number, which is what
    // makes it sensitive to where on the timeline it lands.
    // The shot under test is deliberately not the last one: the last
    // chunk is where the rounding of the whole video is settled, so it
    // is the one chunk that does depend on everything before it.
    let before = plan(
        vec![
            shot("a#0", 0, 1_000, "/clips/a.mp4"),
            shot("b#0", 1_000, 1_030, "/clips/b.mp4"),
            shot("c#0", 2_030, 1_000, "/clips/c.mp4"),
        ],
        3_030,
    );
    // The sentence over the first shot got twenty milliseconds longer.
    let after = plan(
        vec![
            shot("a#0", 0, 1_020, "/clips/a.mp4"),
            shot("b#0", 1_020, 1_030, "/clips/b.mp4"),
            shot("c#0", 2_050, 1_000, "/clips/c.mp4"),
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
        "the second shot shows the same clip for the same length; only \
         everything before it moved"
    );
}

/// The one plan that cannot be chunked is one the scheduler cannot make.
///
/// `chunks` returns `None` for a shot shorter than the transitions either
/// side of it, because the two blends would draw the same frames twice.
/// That used to send the whole build to a second renderer, silently. The
/// scheduler now caps a transition against what the item it arrives in has
/// left (see `a_transition_leaves_room_for_the_one_that_arrived_before_it`),
/// so the case is unreachable from a real script — and the renderer reports
/// it rather than rerouting around it.
#[test]
fn a_hand_built_plan_whose_transitions_overlap_cannot_be_chunked() {
    let plan = plan(
        vec![
            blended("a", 0, 4_000, "a.mp4"),
            blended("b", 3_600, 500, "b.mp4"),
            blended("c", 3_700, 4_000, "c.mp4"),
        ],
        8_000,
    );
    assert!(
        chunk::chunks(&plan).is_none(),
        "the scheduler prevents this shape; chunking must not invent a cut for it"
    );
}

/// A name is drawn into the chunks it shows over, on each one's own clock,
/// and keyed into them; a chunk no one is named over keeps its key.
#[test]
fn a_title_is_drawn_into_the_chunks_it_overlaps_and_only_those() {
    let bare = plan(
        vec![
            shot("a#0", 0, 2_000, "a.mp4"),
            shot("b#0", 2_000, 2_000, "b.mp4"),
        ],
        4_000,
    );
    let titled = |text: &str| RenderPlan {
        titles: vec![teleprompt_render::Title {
            text: text.into(),
            start_ms: TimeMs::at(1_000),
            duration_ms: SpanMs::of(800),
        }],
        ..bare.clone()
    };
    let chunks = chunk::chunks(&titled("Ada")).unwrap();
    assert_eq!(chunks.len(), 2);
    let title = &chunks[0].titles[0];
    // 1 s in, at 25 fps, for 0.8 s.
    assert_eq!((title.from_frame, title.to_frame), (25, 45));
    assert!(chunks[1].titles.is_empty());

    let key = |p: &RenderPlan, i: usize| {
        let c = chunk::chunks(p).unwrap();
        ChunkKey::for_chunk(p, &c[i]).hash(&mut same).unwrap()
    };
    assert_eq!(key(&bare, 1), key(&titled("Ada"), 1));
    assert_ne!(key(&bare, 0), key(&titled("Ada"), 0));
    assert_ne!(key(&titled("Ada"), 0), key(&titled("Charles"), 0));
}
