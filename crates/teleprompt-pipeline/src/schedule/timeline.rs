use serde::{Deserialize, Serialize};
use teleprompt_core::{
    DurationSource, Hash, ItemId, LineId, PolicyKind, ShotId, SpanMs, Tempo, TimeMs,
};
use teleprompt_script::config::Transition;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub version: u32,
    pub script: String,
    pub locale: String,
    pub duration_ms: SpanMs,
    pub generated_by: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub item: ItemId,
    pub start_ms: TimeMs,
    pub duration_ms: SpanMs,
    pub policy: PolicyKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub narration: Option<NarrationEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<ActionEntry>,
    pub transition: Transition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NarrationEntry {
    pub line: LineId,
    pub source_hash: Hash,
    pub audio_hash: Hash,
    pub start_ms: TimeMs,
    pub duration_ms: SpanMs,
    pub duration_source: DurationSource,
    /// Spoken from a recorded take; absent for a synthesized line.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recorded: bool,
    /// The tempo `fit-line` plays it at, in thousandths; absent at 1000.
    /// `duration_ms` is its length at this tempo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo_permille: Option<Tempo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionEntry {
    pub shot: ShotId,
    pub scene: String,
    pub plugin: String,
    /// Hash of this shot's own source alone.
    pub shot_hash: Hash,
    /// Identity of the picture, chained over every earlier shot in the
    /// session; see `docs/design.md#capture-key`.
    pub capture_key: Hash,
    /// From `session="…"`; `None` is the scene's default session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub start_ms: TimeMs,
    pub duration_ms: SpanMs,
    pub duration_source: DurationSource,
}

impl Timeline {
    pub fn entry(&self, item: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.item == item)
    }
}
