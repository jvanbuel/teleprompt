//! The prompter API, version 1 (`docs/design.md#prompter-api-version-1`).
//! The types read what the server sends and ignore what they do not know,
//! as the API asks of its clients.

use serde::Deserialize;

pub const VERSION: &str = "/api/v1";

/// The next word the reader will say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Default)]
pub struct Position {
    pub line: usize,
    pub word: usize,
}

/// `GET /api/v1/script`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct Script {
    pub lines: Vec<Line>,
    pub shots: Vec<Shot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Line {
    pub id: String,
    pub text: String,
    /// Whether the line has a take read from it as it now reads.
    pub recorded: bool,
    /// Whether it has a take of other words: reworded since, to be
    /// recorded again. Absent from servers before it was added.
    #[serde(default)]
    pub stale: bool,
    /// The line as its take was heard to say it, where that is other
    /// words: to keep instead of reading it again.
    #[serde(default)]
    pub said: Option<String>,
    /// The line against `said`, word by word; empty without it.
    #[serde(default)]
    pub said_diff: Vec<crate::said::Change>,
}

impl Line {
    /// The words as the server counts them: split at whitespace.
    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.text.split_whitespace()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Shot {
    pub shot: String,
    pub at: Position,
    /// The clip's path under the server's origin, or none if the shot was
    /// never captured.
    pub clip: Option<String>,
}

/// A message from the server on the session socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMessage {
    /// Where the reader is, and the shots that just reached their cue.
    Reached {
        at: Position,
        play: Vec<String>,
    },
    /// The ids of the lines kept from the take.
    Stopped {
        saved: Vec<String>,
    },
    /// The take was thrown away.
    Discarded,
    /// The lines whose earlier takes were put back.
    Undone {
        lines: Vec<String>,
    },
    Error(String),
    /// A message this client does not know; a later v1 server may send it.
    Unknown(String),
}

impl ServerMessage {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "lowercase")]
        enum Known {
            Reached {
                line: usize,
                word: usize,
                play: Vec<String>,
            },
            Stopped {
                saved: Vec<String>,
            },
            Discarded,
            Undone {
                lines: Vec<String>,
            },
            Error {
                message: String,
            },
        }
        #[derive(Deserialize)]
        struct Header {
            r#type: String,
        }
        let header: Header = serde_json::from_str(json)?;
        let known = ["reached", "stopped", "discarded", "undone", "error"];
        if !known.contains(&header.r#type.as_str()) {
            return Ok(Self::Unknown(header.r#type));
        }
        Ok(match serde_json::from_str(json)? {
            Known::Reached { line, word, play } => Self::Reached {
                at: Position { line, word },
                play,
            },
            Known::Stopped { saved } => Self::Stopped { saved },
            Known::Discarded => Self::Discarded,
            Known::Undone { lines } => Self::Undone { lines },
            Known::Error { message } => Self::Error(message),
        })
    }
}

/// A command to the server on the session socket. Audio goes as binary
/// messages; see [`encode_samples`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMessage {
    /// A new take at line `from`, with audio at `rate` Hz.
    Start { from: usize, rate: u32 },
    /// Keep the lines read in full.
    Stop,
    /// End the take keeping nothing.
    Discard,
    /// Put back what the last take kept replaced.
    Undo,
}

impl ClientMessage {
    pub fn json(&self) -> String {
        match *self {
            Self::Start { from, rate } => {
                serde_json::json!({ "type": "start", "from": from, "rate": rate })
            }
            Self::Stop => serde_json::json!({ "type": "stop" }),
            Self::Discard => serde_json::json!({ "type": "discard" }),
            Self::Undo => serde_json::json!({ "type": "undo" }),
        }
        .to_string()
    }
}

/// Mono samples as the socket takes them: little-endian 32-bit floats.
pub fn encode_samples(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}
