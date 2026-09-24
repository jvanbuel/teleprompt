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
    pub shot: String,
    pub scene: String,
    pub adapter: String,
    /// Hash of this shot's own source alone.
    pub shot_hash: Hash,
    /// Identity of the picture, chained over every earlier shot in the
    /// session; see `docs/design.md#capture-key`.
    pub capture_key: Hash,
    /// From `session="…"`; `None` is the scene's default session.
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
