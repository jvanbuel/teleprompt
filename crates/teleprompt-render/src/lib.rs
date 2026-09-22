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
    pub beats: Vec<Beat>,
    pub narration: Vec<Narration>,
    pub output: PathBuf,
}

/// One scheduled cue of picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beat {
    pub id: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub picture: Picture,
    /// The transition *out* of this beat, as the manifest publishes it.
    ///
    /// Only its kind is read. How long the join lasts is already in the
    /// arithmetic — the scheduler overlaps the next beat's `start_ms` by
    /// exactly the transition it granted — and a renderer that believed
    /// this field instead would be free to disagree with the offsets it is
    /// rendering against.
    pub transition: Transition,
}

/// How one beat gives way to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub kind: String,
    pub duration_ms: u64,
}

impl Transition {
    /// A hard cut, which is what a beat that nothing follows also gets.
    pub fn cut() -> Self {
        Self {
            kind: "cut".into(),
            duration_ms: 0,
        }
    }
}

/// What fills a beat's frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picture {
    /// A captured clip, to be fitted to the beat's scheduled duration.
    Clip(PathBuf),
    /// Nothing was captured for this beat and there is nothing earlier to
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

/// How a [`RenderPlan`] becomes a file.
///
/// A trait rather than a function because there is more than one honest way
/// to do it: ffmpeg handles everything, and a pure-Rust path can handle the
/// case where every beat is a hard cut — concat, mix, mux — which is what
/// would make a single static binary possible for those scripts.
pub trait Renderer {
    /// Stable identifier, for reporting which path a render took.
    fn id(&self) -> &'static str;

    /// Render `plan`, calling `on_progress` as the work proceeds.
    ///
    /// Progress is a callback rather than a returned iterator so a renderer
    /// that has no intermediate state to report is free to call it once, at
    /// the end, and a caller that does not care passes a closure that
    /// discards it.
    fn render(
        &self,
        plan: &RenderPlan,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Rendered, RenderError>;
}
