//! The output cut into independently encodable chunks
//! (`docs/design.md#compose-cache`): a **body** is a window of one
//! placement, and a **blend** is the overlap two placements share.
//!
//! Everything counts in *frames*, and a chunk's length is rounded from its
//! **own duration**, never read off the timeline. Otherwise a line that
//! grows by a fraction of a frame makes later chunks round the other way,
//! and an unchanged shot gets a new key. The price is that the parts need
//! not add up to the whole; the last chunk absorbs the difference.

use std::path::{Path, PathBuf};
use teleprompt_core::config::TransitionKind;
use teleprompt_core::time;

use teleprompt_core::Hash;

use crate::placement::{placements, Placement};
use crate::{Picture, RenderPlan};

/// Not [`Picture`]: a window is never a `Hold`, because placements have
/// already folded holds into the picture before them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Clip(PathBuf),
    Slate,
}

/// `frames` frames of one placement from `from_frame` of its own timeline,
/// whose first `lead_in_frames` are its first frame, frozen.
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
    Body(Window),
    /// The whole of a transition, so it never invalidates the bodies it
    /// joins.
    Blend {
        kind: TransitionKind,
        from: Window,
        to: Window,
    },
}

/// A contiguous run of output frames that can be encoded on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The chunks tile the output with no gap and no overlap.
    pub start_frame: u64,
    pub frames: u64,
    pub content: Content,
}

/// `None` when uncuttable: see [`crate::RenderError::Unchunkable`].
pub fn chunks(plan: &RenderPlan) -> Option<Vec<Chunk>> {
    let fps = plan.fps;
    let placements = placements(plan);
    if placements.is_empty() || fps == 0 {
        return None;
    }

    // A placement's `blend` is its overlap with the one *before* it, so
    // placement i's tail is placement i+1's head.
    let head = |i: usize| placements[i].blend.as_ref().map_or(0, |(_, ms)| *ms);
    let tail = |i: usize| placements.get(i + 1).map_or(0, |_| head(i + 1));
    if (0..placements.len()).any(|i| head(i) + tail(i) > placements[i].duration_ms) {
        return None;
    }

    let mut out = Vec::new();
    let mut cursor = 0u64;
    for (i, placement) in placements.iter().enumerate() {
        if let Some((kind, overlap)) = &placement.blend {
            let previous = &placements[i - 1];
            let (start, end) = (placement.start_ms, placement.start_ms + overlap);
            push(&mut out, &mut cursor, fps, end - start, |frames| {
                Content::Blend {
                    kind: kind.clone(),
                    // The tail of the placement being left…
                    from: window(previous, start, frames, fps),
                    // …against the head of the one arriving.
                    to: window(placement, start, frames, fps),
                }
            });
        }
        let (start, end) = (
            placement.start_ms + head(i),
            placement.start_ms + placement.duration_ms - tail(i),
        );
        push(&mut out, &mut cursor, fps, end - start, |frames| {
            Content::Body(window(placement, start, frames, fps))
        });
    }

    // The whole rounding correction goes on the last chunk, a held frame,
    // so no other chunk's key depends on what came before it.
    settle(&mut out, time::frames(plan.duration_ms, fps));
    Some(out)
}

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

/// Skips a chunk that rounds to no frames; its neighbours cover it.
fn push(
    out: &mut Vec<Chunk>,
    cursor: &mut u64,
    fps: u32,
    duration_ms: u64,
    content: impl FnOnce(u64) -> Content,
) {
    let frames = time::frames(duration_ms, fps);
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

/// Every number is relative to the placement, so a placement that only
/// slid along the timeline keeps the same window and key.
fn window(placement: &Placement, start_ms: u64, frames: u64, fps: u32) -> Window {
    Window {
        source: match &placement.picture {
            Picture::Clip(path) => Source::Clip(path.clone()),
            // `Hold` never reaches here; placements fold it away.
            Picture::Hold | Picture::Slate => Source::Slate,
        },
        lead_in_frames: time::frames(placement.lead_in_ms, fps),
        from_frame: time::frames(start_ms.saturating_sub(placement.start_ms), fps),
        frames,
    }
}

/// Bump when the encoder settings or the filter chain change, or old
/// chunks are served for frames that would now encode differently.
pub(crate) const RECIPE: &str = "x264-crf23-medium-v1";

/// Everything that changes a chunk's bytes, and nothing that does not.
/// A forgotten input serves wrong frames silently, so [`ChunkKey::hash`]
/// destructures this exhaustively: a new field is a compile error there.
#[derive(Debug, Clone, Copy)]
pub struct ChunkKey<'a> {
    pub recipe: &'a str,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub content: &'a Content,
}

impl<'a> ChunkKey<'a> {
    /// Not in the key: the shot's name, its position or the script, so
    /// the same picture is encoded once wherever it appears.
    pub fn for_chunk(plan: &'a RenderPlan, chunk: &'a Chunk) -> Self {
        Self {
            recipe: RECIPE,
            width: plan.width,
            height: plan.height,
            fps: plan.fps,
            content: &chunk.content,
        }
    }

    /// A clip is keyed on its *contents*, never its path: a re-recorded
    /// scene can land at the same path. `clip` reads the file, which keeps
    /// this testable without IO.
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
                fields.push(kind.as_str().to_string());
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
