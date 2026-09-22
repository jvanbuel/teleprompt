//! The output cut into independently encodable chunks.
//!
//! A monolithic render re-encodes every frame of a video to change one
//! sentence of it. Most of those frames are identical to the ones already
//! on disk — the picture under paragraph nine does not move because
//! paragraph two was reworded — and the only thing standing between an
//! author and a one-second rebuild is that nothing names the parts.
//!
//! A chunk is that name. It is a contiguous run of output frames that
//! depends on nothing outside itself, so it can be encoded once, cached
//! under a key covering everything that changes its bytes, and stitched
//! back with `concat -c copy`, which copies compressed frames rather than
//! decoding them.
//!
//! Two shapes are enough to tile any plan. A **body** is a window of one
//! piece. A **blend** is the overlap two pieces share, which is the one
//! region that cannot belong to either of them.
//!
//! Everything here counts in *frames*, not milliseconds, and a chunk's
//! length is rounded from its **own duration** rather than read off the
//! global timeline. That choice is the whole of the cache's usefulness.
//! Reading boundaries off the timeline is more obviously exact, and it was
//! how this worked first — but then a sentence reworded at the top of a
//! script shifts everything after it by some fraction of a frame, half the
//! chunks downstream round the other way, and a rebuild re-encodes more
//! than half a video in which nothing after paragraph two changed. Keyed
//! on its own duration, a beat that did not change does not change.
//!
//! The price is that rounded parts need not add up to the rounded whole,
//! so the last chunk — a held frame at the end of the video, the most
//! forgiving place there is — absorbs the difference.

use std::path::{Path, PathBuf};

use teleprompt_core::Hash;

use crate::piece::{pieces, Piece};
use crate::{Picture, RenderPlan};

/// What a window draws from.
///
/// Deliberately not [`Picture`]: a window can never be a `Hold`, because
/// holding is what [`crate::piece`] resolves before a piece exists. An
/// enum that cannot express the impossible case needs no branch for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Clip(PathBuf),
    Slate,
}

/// `frames` frames of one piece, starting at `from_frame` of that piece's
/// own timeline — in which the first `lead_in_frames` are its first frame,
/// frozen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub source: Source,
    pub lead_in_frames: u64,
    pub from_frame: u64,
    pub frames: u64,
}

/// What a chunk draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// A window of a single piece.
    Body(Window),
    /// Two windows of equal length, blended. This is the whole of a
    /// transition: the frames either side of it are ordinary bodies, which
    /// is what keeps a transition from invalidating the beats it joins.
    Blend {
        kind: String,
        from: Window,
        to: Window,
    },
}

/// A contiguous run of output frames that can be encoded on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Where this chunk begins in the output, in frames. The chunks
    /// tile `[0, frames_of(plan))` with no gap and no overlap.
    pub start_frame: u64,
    pub frames: u64,
    pub content: Content,
}

/// How many frames a duration is, rounded to nearest.
///
/// Always applied to a *duration* — never to an offset on the timeline —
/// so that the answer does not depend on where on the timeline it falls.
fn frames_of(ms: u64, fps: u32) -> u64 {
    (ms * u64::from(fps) + 500) / 1000
}

/// `plan` as chunks, or `None` if it cannot be cut into any.
///
/// The one plan that cannot is a beat shorter than the transitions either
/// side of it: the two blends would draw the same frames twice. That is
/// rare enough to be worth reporting rather than engineering around —
/// the caller renders the whole graph in one pass instead, which is what
/// it did before chunks existed.
pub fn chunks(plan: &RenderPlan) -> Option<Vec<Chunk>> {
    let fps = plan.fps;
    let pieces = pieces(plan);
    if pieces.is_empty() || fps == 0 {
        return None;
    }

    // How much each piece gives up at each end. A piece's `blend` is the
    // overlap it shares with the piece *before* it, so the tail of piece i
    // is the head of piece i+1.
    let head = |i: usize| pieces[i].blend.as_ref().map_or(0, |(_, ms)| *ms);
    let tail = |i: usize| pieces.get(i + 1).map_or(0, |_| head(i + 1));
    if (0..pieces.len()).any(|i| head(i) + tail(i) > pieces[i].duration_ms) {
        return None;
    }

    let mut out = Vec::new();
    let mut cursor = 0u64;
    for (i, piece) in pieces.iter().enumerate() {
        if let Some((kind, overlap)) = &piece.blend {
            let previous = &pieces[i - 1];
            let (start, end) = (piece.start_ms, piece.start_ms + overlap);
            push(&mut out, &mut cursor, fps, end - start, |frames| {
                Content::Blend {
                    kind: kind.clone(),
                    // The tail of the piece being left…
                    from: window(previous, start, frames, fps),
                    // …against the head of the one arriving.
                    to: window(piece, start, frames, fps),
                }
            });
        }
        let (start, end) = (
            piece.start_ms + head(i),
            piece.start_ms + piece.duration_ms - tail(i),
        );
        push(&mut out, &mut cursor, fps, end - start, |frames| {
            Content::Body(window(piece, start, frames, fps))
        });
    }

    // Rounded parts need not add up to the rounded whole. The last chunk
    // holds a frame at the end of the video, so it is where a few
    // thousandths of a second cost the least — and putting the whole
    // correction in one place is what keeps every other chunk's key
    // independent of what happened earlier in the script.
    settle(&mut out, frames_of(plan.duration_ms, fps));
    Some(out)
}

/// Stretch or shorten the last chunk so the chunks total `target`.
fn settle(out: &mut [Chunk], target: u64) {
    let Some(last) = out.last_mut() else {
        return;
    };
    let total = last.start_frame + last.frames;
    let frames = (last.frames + target).saturating_sub(total).max(1);
    last.frames = frames;
    match &mut last.content {
        Content::Body(window) => window.frames = frames,
        Content::Blend { from, to, .. } => {
            from.frames = frames;
            to.frames = frames;
        }
    }
}

/// Append a chunk of `duration_ms`, unless it rounds to no frames at
/// all — a beat too short to draw a frame of, which the chunks either
/// side of it already cover.
fn push(
    out: &mut Vec<Chunk>,
    cursor: &mut u64,
    fps: u32,
    duration_ms: u64,
    content: impl FnOnce(u64) -> Content,
) {
    let frames = frames_of(duration_ms, fps);
    if frames == 0 {
        return;
    }
    out.push(Chunk {
        start_frame: *cursor,
        frames,
        content: content(frames),
    });
    *cursor += frames;
}

/// `frames` frames of `piece`, starting at output time `start_ms`.
///
/// Every number in here is a duration — how far into the piece the window
/// begins, how long its lead-in is, how long it lasts — so a piece that
/// slid along the timeline without otherwise changing produces the same
/// window, and the same key.
fn window(piece: &Piece, start_ms: u64, frames: u64, fps: u32) -> Window {
    Window {
        source: match &piece.picture {
            Picture::Clip(path) => Source::Clip(path.clone()),
            // `Hold` cannot reach here: a held beat is folded into the
            // piece before it, which is what holding means.
            Picture::Hold | Picture::Slate => Source::Slate,
        },
        lead_in_frames: frames_of(piece.lead_in_ms, fps),
        from_frame: frames_of(start_ms.saturating_sub(piece.start_ms), fps),
        frames,
    }
}

/// The recipe version: bumped when the encoder settings or the filter
/// chain change, so a cache filled by an older teleprompt is not served
/// for frames a newer one would encode differently.
pub const RECIPE: &str = "x264-crf23-medium-v1";

/// Everything that changes a chunk's bytes, and nothing that does not.
///
/// The whole risk of an input-addressed key is the input somebody forgot:
/// it does not fail, it serves the wrong frames. So this is one struct,
/// and [`ChunkKey::hash`] destructures it exhaustively — a field added
/// here is a compile error there rather than something to remember.
#[derive(Debug, Clone, Copy)]
pub struct ChunkKey<'a> {
    pub recipe: &'a str,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub content: &'a Content,
}

impl<'a> ChunkKey<'a> {
    /// The key for `chunk` rendered under `plan`'s geometry.
    ///
    /// Note what is *not* in it: the beat's name, its place in the script,
    /// the script itself. Two scripts that put the same picture in the same
    /// place encode it once.
    pub fn for_chunk(plan: &'a RenderPlan, chunk: &'a Chunk) -> Self {
        Self {
            recipe: RECIPE,
            width: plan.width,
            height: plan.height,
            fps: plan.fps,
            content: &chunk.content,
        }
    }

    /// The key, with clip identity resolved by `clip`.
    ///
    /// A clip is keyed on its *contents*, never its path: capture writes a
    /// re-recorded scene back to the same filename, and a key on the path
    /// would serve the old frames for ever. Resolving is the caller's job
    /// because it reads files, and this has to stay testable without any.
    pub fn hash(
        &self,
        clip: &mut dyn FnMut(&Path) -> std::io::Result<Hash>,
    ) -> std::io::Result<Hash> {
        let ChunkKey {
            recipe,
            width,
            height,
            fps,
            content,
        } = self;
        let mut fields = vec![
            recipe.to_string(),
            width.to_string(),
            height.to_string(),
            fps.to_string(),
        ];
        match content {
            Content::Body(window) => {
                fields.push("body".into());
                push_window(&mut fields, window, clip)?;
            }
            Content::Blend { kind, from, to } => {
                fields.push("blend".into());
                fields.push(kind.clone());
                push_window(&mut fields, from, clip)?;
                push_window(&mut fields, to, clip)?;
            }
        }
        Ok(Hash::of_fields(
            &fields.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
    }
}

fn push_window(
    fields: &mut Vec<String>,
    window: &Window,
    clip: &mut dyn FnMut(&Path) -> std::io::Result<Hash>,
) -> std::io::Result<()> {
    let Window {
        source,
        lead_in_frames,
        from_frame,
        frames,
    } = window;
    fields.push(match source {
        Source::Clip(path) => format!("clip:{}", clip(path)?),
        Source::Slate => "slate".into(),
    });
    fields.push(lead_in_frames.to_string());
    fields.push(from_frame.to_string());
    fields.push(frames.to_string());
    Ok(())
}
