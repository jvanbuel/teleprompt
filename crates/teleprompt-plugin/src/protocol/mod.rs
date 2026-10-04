//! Plugins as programs of their own (`docs/guide/plugins.md#the-protocol`).
//!
//! A plugin is an executable named `teleprompt-adapter-<name>` or
//! `teleprompt-voice-<name>`, on PATH or in the plugins directory. Teleprompt
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

use crate::voice::WordTiming;

pub mod host;
pub mod serve;

/// The protocol's version, which `describe` is asked for and answers with.
pub const VERSION: u32 = 1;

/// What a plugin is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Adapter,
    Voice,
}

impl Kind {
    /// The prefix its executable's name starts with.
    pub fn prefix(self) -> &'static str {
        match self {
            Kind::Adapter => "teleprompt-adapter-",
            Kind::Voice => "teleprompt-voice-",
        }
    }
}

/// `describe`: what the plugin is and needs, asked once, first.
///
/// The two kinds share only this: a name, the protocol, and the tools they
/// need. The rest is the kind's own, beside them on the wire, tagged by
/// `kind`: `{"kind": "adapter", "continues": false, …}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Description {
    pub protocol: u32,
    /// The name a block's `scene=` or a project's `voice.backend` gives,
    /// which its executable's name ends with.
    pub name: String,
    #[serde(default)]
    pub needs: Vec<WireTool>,
    #[serde(flatten)]
    pub traits: Traits,
}

/// What one kind of plugin says of itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Traits {
    Adapter(AdapterTraits),
    Voice(VoiceTraits),
}

impl Description {
    pub fn kind(&self) -> Kind {
        match self.traits {
            Traits::Adapter(_) => Kind::Adapter,
            Traits::Voice(_) => Kind::Voice,
        }
    }
}

fn yes() -> bool {
    true
}

/// How an adapter's scenes behave, beyond its methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterTraits {
    /// Whether a shot opens on the screen the previous one left.
    #[serde(default = "yes")]
    pub continues: bool,
    /// Whether it answers `retime`.
    #[serde(default)]
    pub retimes: bool,
}

impl Default for AdapterTraits {
    fn default() -> Self {
        Self {
            continues: true,
            retimes: false,
        }
    }
}

/// What a voice can do, beyond speaking a line.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoiceTraits {
    #[serde(default)]
    pub word_timings: bool,
    #[serde(default)]
    pub speed_control: bool,
    /// Where its server is, for errors and `setup`.
    #[serde(default)]
    pub address: Option<String>,
    /// Whether it answers `voices`, which `dub` checks a script's against.
    #[serde(default)]
    pub lists_voices: bool,
    /// Whether it answers `probe`, the line `setup` prints.
    #[serde(default)]
    pub probes: bool,
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

/// `validate` and `shots`: a block's body, in the adapter's language.
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

/// `configure`: a voice's own `[backends.<name>]` settings, before
/// anything else is asked of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Configure {
    #[serde(default)]
    pub settings: Option<serde_json::Value>,
}

/// `configure`'s answer: what the audio depends on beyond the request (a
/// model, say), which is part of the voice cache's key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Configured {
    pub version: String,
}

/// `synthesize`: a line, and the WAV file to write it to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Synthesize {
    pub text: String,
    pub locale: String,
    #[serde(default)]
    pub voice: Option<String>,
    pub speed: f64,
    #[serde(default)]
    pub instruct: Option<String>,
    pub out: PathBuf,
}

/// `synthesize`'s answer, once the WAV is written.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Spoken {
    #[serde(default)]
    pub word_timings: Option<Vec<WordTiming>>,
}

/// `voices`' answer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Voices {
    pub voices: Vec<String>,
}

/// `probe`'s answer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Probed {
    pub line: String,
}
