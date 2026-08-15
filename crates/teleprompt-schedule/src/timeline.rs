use serde::Serialize;
use teleprompt_core::Hash;

#[derive(Debug, Clone, Serialize)]
pub struct Timeline {
    pub version: u32,
    pub script: String,
    pub locale: String,
    pub duration_ms: u64,
    pub generated_by: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub beat: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub narration: Option<NarrationEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<ActionEntry>,
    pub transition: TransitionEntry,
}

#[derive(Debug, Clone, Serialize)]
pub struct NarrationEntry {
    pub segment: String,
    pub source_hash: Hash,
    pub audio_hash: Hash,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub voice_source: String,
    pub voice_source_actual: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downgrade_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionEntry {
    pub span: String,
    pub scene: String,
    pub adapter: String,
    pub span_hash: Hash,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub duration_source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransitionEntry {
    pub kind: String,
    pub duration_ms: u64,
}

impl Timeline {
    pub fn entry(&self, beat: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.beat == beat)
    }
}
