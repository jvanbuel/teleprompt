//! Scene plugins as programs of their own
//! (`docs/guide/scene-plugins.md#the-protocol`).
//!
//! A plugin is an executable named `teleprompt-scene-<name>`, on PATH or in
//! the plugins directory. Teleprompt
//! starts it the first time a command needs it and talks to it for the rest
//! of that command: one JSON object a line, a request on its stdin and the
//! answer on its stdout. Its stderr is the author's to read.
//!
//! ```text
//! → {"id":1,"method":"shots","params":{"scene":"card","body":"…"}}
//! ← {"id":1,"progress":{…}}          (capture only, any number)
//! ← {"id":1,"result":{"shots":[…]}}  or  {"id":1,"error":"why"}
//! ```
//!
//! [`host`] is teleprompt's side; [`serve`] is a Rust plugin's, so a crate
//! written against the contracts can be built as an executable too.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::capture::{Clip, Frame, Session};
use crate::scene::Measured;

pub mod host;
pub mod serve;

/// The protocol's version, which `describe` is asked for and answers with.
pub const VERSION: u32 = 1;

/// What a plugin's executable is named: this, then its name.
pub const PREFIX: &str = "teleprompt-scene-";

/// `describe`: what the plugin is and needs, asked once, first:
/// `{"protocol": 1, "name": "card", "needs": […], "continues": false}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Description {
    pub protocol: u32,
    /// The name a scene's `plugin` gives, which its executable's name ends
    /// with.
    pub name: String,
    /// What it runs. Teleprompt lists them in `setup`, and a program among
    /// them that is not on PATH is why the plugin cannot capture here.
    #[serde(default)]
    pub needs: Vec<Need>,
    /// Whether a shot opens on the screen the previous one left.
    #[serde(default = "yes")]
    pub continues: bool,
}

fn yes() -> bool {
    true
}

/// The error a plugin answers for a method it does not have.
pub fn unknown_method(method: &str) -> String {
    format!("unknown method `{method}`")
}

/// A tool a plugin needs: [`crate::tool::Tool`], owned, as a description
/// carries it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Need {
    pub name: String,
    pub what: String,
    pub license: String,
    pub home: String,
    #[serde(default)]
    pub guide: Option<String>,
    /// The program that shows it is installed.
    #[serde(default)]
    pub program: Option<String>,
    /// Or the npm package, in the project.
    #[serde(default)]
    pub package: Option<String>,
    /// The command per package manager: `brew`, `apt`, `dnf`, `pacman`,
    /// `go`, `cargo`, `pipx`, `npm`.
    #[serde(default)]
    pub install: BTreeMap<String, String>,
}

/// `validate` and `shots`: a block's body, in the plugin's language.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Body {
    pub scene: String,
    pub body: String,
}

/// `validate`'s answer: every bad line, none for a good block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Validation {
    #[serde(default)]
    pub errors: Vec<LineError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineError {
    /// The body line, from 0; none for the block as a whole.
    #[serde(default)]
    pub line: Option<usize>,
    pub message: String,
    #[serde(default)]
    pub help: Option<String>,
}

/// `shots`' answer: the block split at its marks, in order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Shots {
    pub shots: Vec<Part>,
}

/// One shot as `shots` answers it: `{"source": …, "ms": …, "exact": …}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Part {
    pub source: String,
    #[serde(flatten)]
    pub length: Measured,
}

/// `retime`: a shot's source, to last exactly `target_ms`. A plugin that
/// cannot re-time answers it with an error, or null.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Retime {
    pub source: String,
    pub target_ms: u64,
}

/// `retime`'s answer: the rewritten source, or none when it cannot be.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Retimed {
    #[serde(default)]
    pub source: Option<String>,
}

/// `capture`: one session, the size to make it, and where its clips go.
/// Progress events are [`crate::capture::Progress`]es; the answer is [`Captured`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capture {
    pub session: Session,
    pub frame: Frame,
    pub out_dir: PathBuf,
}

/// `capture`'s answer: a clip for each wanted shot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Captured {
    pub clips: Vec<Clip>,
}
