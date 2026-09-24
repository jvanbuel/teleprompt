use serde::{Deserialize, Serialize};
use teleprompt_core::{DurationSource, Hash, PolicyKind, VoiceSource, VoiceTier};

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
    pub policy: PolicyKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub narration: Option<NarrationEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<ActionEntry>,
    pub transition: TransitionEntry,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "NarrationWire", try_from = "NarrationWire")]
pub struct NarrationEntry {
    pub line: String,
    pub source_hash: Hash,
    pub audio_hash: Hash,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub duration_source: DurationSource,
    pub voice: VoiceTier,
}

/// [`NarrationEntry`] as the timeline writes it: the voice tier as three
/// fields, the reason omitted when there is none.
#[derive(Clone, Serialize, Deserialize)]
struct NarrationWire {
    line: String,
    source_hash: Hash,
    audio_hash: Hash,
    start_ms: u64,
    duration_ms: u64,
    duration_source: DurationSource,
    voice_source: VoiceSource,
    voice_source_actual: VoiceSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    downgrade_reason: Option<String>,
}

impl From<NarrationEntry> for NarrationWire {
    fn from(e: NarrationEntry) -> Self {
        Self {
            voice_source: e.voice.requested(),
            voice_source_actual: e.voice.actual(),
            downgrade_reason: e.voice.reason().map(str::to_string),
            line: e.line,
            source_hash: e.source_hash,
            audio_hash: e.audio_hash,
            start_ms: e.start_ms,
            duration_ms: e.duration_ms,
            duration_source: e.duration_source,
        }
    }
}

impl TryFrom<NarrationWire> for NarrationEntry {
    type Error = String;

    fn try_from(w: NarrationWire) -> Result<Self, String> {
        Ok(Self {
            voice: VoiceTier::from_fields(
                w.voice_source,
                w.voice_source_actual,
                w.downgrade_reason,
            )?,
            line: w.line,
            source_hash: w.source_hash,
            audio_hash: w.audio_hash,
            start_ms: w.start_ms,
            duration_ms: w.duration_ms,
            duration_source: w.duration_source,
        })
    }
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
    pub duration_source: DurationSource,
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
