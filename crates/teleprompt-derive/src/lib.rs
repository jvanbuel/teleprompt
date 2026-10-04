//! Deriving a script from a recorded session: what was said and when,
//! and when each step of the recording began, become narration lines and
//! the blocks between them (`docs/design.md#recording-a-session`).
//!
//! Pure: the caller reads the recording, as the adapter that made it
//! does, and transcribes the voice. A block names a run of steps; the
//! script includes that part of the recording.
//!
//! A script is also drafted from what already exists: a Markdown document
//! or a Slidev deck's speaker notes ([`document`]), or the transcript of a
//! conversation ([`transcript`]).

mod beats;
pub mod document;
mod markdown;
mod punctuate;
pub mod transcript;

pub use beats::{derive, Beat, Block, Line, Mode};
pub use punctuate::punctuate;

/// A word the recognizer heard, and when, in milliseconds from the start
/// of the trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// How a session is cut into a script.
#[derive(Debug, Clone)]
pub struct Options {
    /// A silence at least this long ends a line.
    pub pause_ms: u64,
    /// The scene the blocks run in: the recording adapter's own.
    pub scene: String,
    /// The recording, as the script's `include=` names it.
    pub include: String,
    /// The script's one heading.
    pub title: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pause_ms: 700,
            scene: "asciinema".to_string(),
            include: "recordings/session.cast".to_string(),
            title: "Recording".to_string(),
        }
    }
}

/// A derived script: its lines, each with the blocks that follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub beats: Vec<Beat>,
}

impl Draft {
    /// The script as Markdown, a first draft for the author to edit.
    pub fn markdown(&self, options: &Options) -> String {
        markdown::render(self, options)
    }

    /// Its narration lines, in order.
    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.beats.iter().filter_map(|b| b.line.as_ref())
    }

    /// Its blocks, in order: block `n` includes part `n + 1` of the
    /// recording.
    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.beats.iter().flat_map(|b| &b.blocks)
    }

    /// Where the recording is cut into those parts: before each block's
    /// first step but the first block's.
    pub fn cuts(&self) -> Vec<usize> {
        self.blocks().skip(1).map(|b| b.steps.start).collect()
    }
}
