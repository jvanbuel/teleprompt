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

pub mod host;
pub mod serve;

/// The protocol's version, which `describe` is asked for and answers with.
pub const VERSION: u32 = 1;

/// What a plugin's executable is named: this, then its name.
pub const PREFIX: &str = "teleprompt-scene-";

/// `describe`: what the plugin is and needs, asked once, first:
/// `{"protocol": 1, "name": "card", "needs": […], "continues": false, …}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Description {
    pub protocol: u32,
    /// The name a scene's `plugin` gives, which its executable's name ends
    /// with.
    pub name: String,
    #[serde(default)]
    pub needs: Vec<WireTool>,
    #[serde(flatten)]
    pub traits: SceneTraits,
}

fn yes() -> bool {
    true
}

/// How a scene plugin's scenes behave, beyond its methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTraits {
    /// Whether a shot opens on the screen the previous one left.
    #[serde(default = "yes")]
    pub continues: bool,
    /// Whether it answers `retime`.
    #[serde(default)]
    pub retimes: bool,
}

impl Default for SceneTraits {
    fn default() -> Self {
        Self {
            continues: true,
            retimes: false,
        }
    }
}

/// The error a plugin answers for a method it does not have.
pub fn unknown_method(method: &str) -> String {
    format!("unknown method `{method}`")
}

/// A tool a plugin needs, as [`crate::tool::Tool`] has it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireTool {
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
    pub shots: Vec<WireShot>,
}

/// A shot's source and how long it takes: `ms` with `exact` when the source
/// states it, `ms` alone for an estimate, neither when it should last as
/// long as its line. `estimate` answers with the same, for one source.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireShot {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub ms: Option<u64>,
    #[serde(default)]
    pub exact: bool,
}

/// `retime`: a shot's source, to last exactly `target_ms`.
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

/// `unavailable`'s answer: why it cannot capture here, or none.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Unavailable {
    #[serde(default)]
    pub reason: Option<String>,
}

/// `capture`: one session, the size to make it, and where its clips go.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capture {
    pub session: WireSession,
    pub frame: WireFrame,
    pub out_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireSession {
    pub scene: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    /// What a relative path in `settings` is relative to: the project's
    /// directory.
    #[serde(default)]
    pub root: PathBuf,
    pub shots: Vec<WireSessionShot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireSessionShot {
    pub id: String,
    /// What its clip is filed under: 64 hex characters.
    pub key: String,
    pub source: String,
    pub duration_ms: u64,
    /// Whether a clip is wanted; a shot whose clip is cached runs anyway.
    pub wanted: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WireFrame {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// A `capture` progress event: a shot done.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireProgress {
    pub shot: String,
    pub done: usize,
    pub of: usize,
}

/// `capture`'s answer: a clip for each wanted shot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Captured {
    pub clips: Vec<WireClip>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireClip {
    pub key: String,
    pub path: PathBuf,
}
