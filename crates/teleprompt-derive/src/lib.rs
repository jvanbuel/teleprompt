//! Deriving a script from a recorded terminal session: what was typed and
//! when, and what was said and when, become narration lines and the tapes
//! that run between them (`docs/design.md#recording-a-session`).
//!
//! Pure: the caller reads the recording and transcribes the voice.

mod beats;
mod cast;
mod keys;
mod markdown;
mod punctuate;
mod tape;

pub use beats::{derive, Beat, Block, Line, Mode};
pub use cast::{read_cast, Trace};
pub use keys::{decode, Key};
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
    /// The scene the tapes run in: VHS's own, unless the project declares
    /// another.
    pub scene: String,
    /// The script's one heading.
    pub title: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pause_ms: 700,
            scene: "vhs".to_string(),
            title: "Recording".to_string(),
        }
    }
}

/// A derived script: its lines, each with the tapes that follow it.
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
}
