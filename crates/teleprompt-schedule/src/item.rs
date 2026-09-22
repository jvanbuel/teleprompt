use teleprompt_core::config::Config;
use teleprompt_core::{Hash, VoiceSource};

use crate::policy::Policy;

#[derive(Debug, Clone)]
pub struct NarrationInput {
    pub line_id: String,
    pub source_hash: Hash,
    pub audio_hash: Hash,
    /// Clip duration excluding lead-in and tail padding.
    pub duration_ms: u64,
    /// Whether `duration_ms` was measured from real audio or predicted.
    ///
    /// `plan` and `diff` never synthesize, so a line not yet in the cache
    /// carries an estimate. Publishing which is which is the difference
    /// between a timeline a reader can trust and one that quietly conflates
    /// a prediction with a measurement.
    pub duration_source: DurationSource,
    /// Silence before the clip, and silence after it.
    ///
    /// These travel with the narration rather than being read off the
    /// [`Item`]'s single `config` because a item's two halves resolve from
    /// different configuration layers: the narration's padding comes from the
    /// *line's* attributes (`{#a lead_in=1000ms}`), while the item's
    /// `config` is the following action block's. Reading padding off
    /// `Item::config` silently discarded every line-level `lead_in=` /
    /// `tail=` whenever an action block followed the paragraph — which, per
    /// spec §3.1, is the normal case.
    pub lead_in_ms: u64,
    pub tail_ms: u64,
    pub voice_source: VoiceSource,
    pub voice_source_actual: VoiceSource,
    pub downgrade_reason: Option<String>,
}

impl NarrationInput {
    /// Total time this narration occupies in its item: lead-in, clip, tail.
    ///
    /// Saturating, not because any validated input can reach `u64::MAX` —
    /// `Config::problems` rejects the `voice.speed` that used to produce one
    /// — but because the scheduler must not panic on an arithmetic edge for
    /// *any* `u64` triple it is handed. A plain add panics in a debug build
    /// and, worse, wraps silently in a release one: a saturated timeline is
    /// visibly absurd, a wrapped one is quietly wrong.
    pub fn padded_duration_ms(&self) -> u64 {
        self.lead_in_ms
            .saturating_add(self.duration_ms)
            .saturating_add(self.tail_ms)
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
    pub shot_id: String,
    pub scene: String,
    pub adapter: String,
    pub shot_hash: Hash,
    pub duration_ms: u64,
    pub duration_source: DurationSource,
    /// Where inside the narration this action should start, from `cue="…"`.
    ///
    /// `None` is the ordinary case: the policy decides. A shot is how an
    /// author says "type the command while the voice is saying it", which
    /// no policy can work out on its own — the sentence that names a
    /// command is rarely the first one in the paragraph.
    pub cue_ms: Option<u64>,
    /// Which run of the scene this action belongs to, from `session="…"`.
    /// `None` is the scene's own, which is where most items live.
    pub session: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub narration: Option<NarrationInput>,
    pub action: Option<ActionInput>,
    pub policy: Policy,
    pub config: Config,
}
