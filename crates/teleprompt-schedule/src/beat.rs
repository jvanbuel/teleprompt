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
    /// Silence before the clip, and silence after it.
    ///
    /// These travel with the narration rather than being read off the
    /// [`Beat`]'s single `config` because a beat's two halves resolve from
    /// different configuration layers: the narration's padding comes from the
    /// *segment's* attributes (`{#a lead_in=1000ms}`), while the beat's
    /// `config` is the following action block's. Reading padding off
    /// `Beat::config` silently discarded every segment-level `lead_in=` /
    /// `tail=` whenever an action block followed the paragraph — which, per
    /// spec §3.1, is the normal case.
    pub lead_in_ms: u64,
    pub tail_ms: u64,
    pub voice_source: VoiceSource,
    pub voice_source_actual: VoiceSource,
    pub downgrade_reason: Option<String>,
}

impl NarrationInput {
    /// Total time this narration occupies in its beat: lead-in, clip, tail.
    pub fn padded_duration_ms(&self) -> u64 {
        self.lead_in_ms + self.duration_ms + self.tail_ms
    }
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
