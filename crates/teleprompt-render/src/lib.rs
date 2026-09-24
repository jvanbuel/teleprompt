//! Composing a rendered video from a published narration manifest.
//!
//! This is stage 6 of the pipeline — everything before it decided *when*
//! each thing happens, and this crate turns those decisions into a file.
//! It deliberately consumes the published manifest rather than an
//! in-process `Timeline`: two timing paths drift, and the one that drifts
//! silently is the one nobody renders from.

pub mod chunk;
pub mod ffmpeg;
pub mod incremental;
mod placement;
pub mod plan;

use std::path::PathBuf;

/// Everything a renderer needs, with every offset already decided.
///
/// A renderer does no scheduling. If a number is wrong here, it was wrong
/// in the manifest, which is the property that makes a preview and a render
/// agree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPlan {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration_ms: u64,
    pub shots: Vec<Shot>,
    pub narration: Vec<Narration>,
    pub output: PathBuf,
}

/// One scheduled shot of picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub id: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub picture: Picture,
    /// The transition *out* of this shot, as the manifest publishes it.
    ///
    /// Only its kind is read. How long the join lasts is already in the
    /// arithmetic — the scheduler overlaps the next shot's `start_ms` by
    /// exactly the transition it granted — and a renderer that believed
    /// this field instead would be free to disagree with the offsets it is
    /// rendering against.
    pub transition: Transition,
}

/// How one shot gives way to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub kind: String,
    pub duration_ms: u64,
}

impl Transition {
    /// A hard cut, which is what a shot that nothing follows also gets.
    pub fn cut() -> Self {
        Self {
            kind: "cut".into(),
            duration_ms: 0,
        }
    }
}

/// What fills a shot's frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picture {
    /// A captured clip, to be fitted to the shot's scheduled duration.
    Clip(PathBuf),
    /// Nothing was captured for this shot and there is nothing earlier to
    /// hold — the opening of a video whose scenes were never captured. A
    /// flat field of the background colour, so the timing is exercised and
    /// the absence is visible rather than silently skipped.
    Slate,
    /// Whatever is already on screen, held. A pause is one of these, and so
    /// is every stretch of narration with no action under it: on a real
    /// script that is most of the running time, and cutting to a blank
    /// field for it is the difference between a video and a slideshow of
    /// black.
    Hold,
}

/// One narration clip and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Narration {
    pub id: String,
    pub path: PathBuf,
    pub start_ms: u64,
}

/// A rendered file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub path: PathBuf,
    /// What the renderer was asked to produce, not what the file measures.
    /// Probing the result is the caller's business, and needs a prober.
    pub duration_ms: u64,
    /// How much of the picture came from a cache of already-encoded frames
    /// rather than from the encoder. `None` from a renderer that has no
    /// cache — which is not the same claim as `Some(0)`, a cold cache.
    pub reused_ms: Option<u64>,
}

/// How far a render has got, reported as it goes rather than at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub rendered_ms: u64,
    pub of_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// A shot shorter than the transitions either side of it, so the two
    /// blends would draw the same frames twice.
    ///
    /// The scheduler caps a transition against what the item it arrives in
    /// has left, so a plan compiled from a script cannot be this shape. It
    /// is reported rather than worked around: the alternative was a second
    /// renderer nobody could see being used.
    #[error(
        "a shot is shorter than the transitions either side of it, so the \
         plan cannot be cut into chunks; this should not be reachable from \
         a script — please report it"
    )]
    Unchunkable,
    #[error("{program} is not available: {source}")]
    Unavailable {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} exited {status}:\n{stderr}")]
    Failed {
        program: String,
        status: String,
        stderr: String,
    },
    #[error("cannot render to {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
