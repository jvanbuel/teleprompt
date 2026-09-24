//! Composing a video from a published narration manifest, not an
//! in-process `Timeline` (`docs/design.md#rendering`).

pub mod chunk;
pub mod ffmpeg;
pub mod incremental;
mod placement;
pub mod plan;

use std::path::PathBuf;
use teleprompt_core::config::TransitionKind;

/// Every offset is already decided: a renderer does no scheduling, so a
/// wrong number here was wrong in the manifest too.
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
    /// The transition *out* of this shot. Only its kind is read: its length
    /// is how far the next shot's `start_ms` overlaps this one, so the
    /// offsets stay the single source of truth.
    pub transition: Transition,
}

/// How one shot gives way to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration_ms: u64,
}

impl Transition {
    /// A hard cut, which is what a shot that nothing follows also gets.
    pub fn cut() -> Self {
        Self {
            kind: TransitionKind::Cut,
            duration_ms: 0,
        }
    }
}

/// What fills a shot's frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picture {
    /// A captured clip, fitted to the shot's duration.
    Clip(PathBuf),
    /// No clip was captured: a flat field of the background colour, so the
    /// absence is visible and later shots keep their places.
    Slate,
    /// Whatever is already on screen, held; a pause is one of these.
    Hold,
}

/// One narration clip and where it starts.
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
    /// What was asked for, not what the file measures.
    pub duration_ms: u64,
    /// Picture served from the chunk cache. `None` means reuse was off,
    /// which is not the same claim as `Some(0)`, a cold cache.
    pub reused_ms: Option<u64>,
}

/// How far a render has got, reported as it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub rendered_ms: u64,
    pub of_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// A shot shorter than the transitions either side of it, so the two
    /// blends would draw the same frames twice. The scheduler caps
    /// transitions, so a compiled script cannot produce this.
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
