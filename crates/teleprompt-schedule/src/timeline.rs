use serde::{Deserialize, Serialize};
use teleprompt_core::Hash;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub version: u32,
    pub script: String,
    pub locale: String,
    pub duration_ms: u64,
    pub generated_by: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub item: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub narration: Option<NarrationEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<ActionEntry>,
    pub transition: TransitionEntry,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NarrationEntry {
    pub line: String,
    pub source_hash: Hash,
    pub audio_hash: Hash,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub duration_source: String,
    pub voice_source: String,
    pub voice_source_actual: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downgrade_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionEntry {
    pub cue: String,
    pub scene: String,
    pub adapter: String,
    /// Identity of this cue's own source — what the author wrote, and
    /// nothing else. What `diff` reads to say *this block changed*.
    pub cue_hash: Hash,
    /// Identity of the picture: this cue's source and every cue before
    /// it in the same session.
    ///
    /// A scene is a session — the items of a walkthrough continue one
    /// another, and the screen at item N is the accumulation of items
    /// 1..N — so a clip is not named by its own tape. Two blocks with the
    /// same steps in the same session show different screens, and naming
    /// them both by `cue_hash` would serve one's picture for the other.
    ///
    /// The arithmetic is OCI's chain ID, for the same reason OCI has one:
    /// `chain(0) = H(name(0))`, `chain(n) = H(chain(n-1) ‖ name(n))`.
    /// Invalidation falls out of it — editing a item invalidates that item
    /// and everything after it in its session, and nothing before it, and
    /// nothing in any other scene.
    pub capture_key: Hash,
    /// Which run of the scene, from `session="…"`. Published because it is
    /// the only part of the chain a reader cannot see in the script.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub duration_source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitionEntry {
    pub kind: String,
    pub duration_ms: u64,
}

impl Timeline {
    pub fn entry(&self, item: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.item == item)
    }
}
