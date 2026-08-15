use teleprompt_core::config::Config;
use teleprompt_core::{Hash, VoiceSource};

use crate::policy::Policy;

#[derive(Debug, Clone)]
pub struct NarrationInput {
    pub segment_id: String,
    pub source_hash: Hash,
    pub audio_hash: Hash,
    /// Clip duration excluding lead-in and tail padding.
    pub duration_ms: u64,
    pub voice_source: VoiceSource,
    pub voice_source_actual: VoiceSource,
    pub downgrade_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationSource {
    Exact,
    Estimated,
    Measured,
}

#[derive(Debug, Clone)]
pub struct ActionInput {
    pub span_id: String,
    pub scene: String,
    pub adapter: String,
    pub span_hash: Hash,
    pub duration_ms: u64,
    pub duration_source: DurationSource,
}

#[derive(Debug, Clone)]
pub struct Beat {
    pub id: String,
    pub narration: Option<NarrationInput>,
    pub action: Option<ActionInput>,
    pub policy: Policy,
    pub config: Config,
}
