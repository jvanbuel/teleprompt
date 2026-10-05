use teleprompt_core::config::{Config, TimingConfig, TransitionConfig};
use teleprompt_core::policy::Align;
use teleprompt_core::{DurationMs, DurationSource, Hash, ItemId, LineId, PolicyKind, ShotId};

#[derive(Debug, Clone)]
pub struct NarrationInput {
    pub line_id: LineId,
    pub source_hash: Hash,
    pub audio_hash: Hash,
    /// Clip duration excluding lead-in and tail padding.
    pub duration_ms: u64,
    /// Measured from audio, or predicted on a cache miss; see
    /// `docs/design.md#estimated-and-measured`.
    pub duration_source: DurationSource,
    /// Silence before and after the clip. Carried here, not read from
    /// [`Item::pacing`], because that is the action block's configuration
    /// layer and would drop the line's own `lead_in=` / `tail=`.
    pub lead_in_ms: DurationMs,
    pub tail_ms: DurationMs,
    /// Spoken from a recorded take rather than synthesized.
    pub recorded: bool,
    /// How many words it says: what `fit-line` counts in when a line is
    /// too long for its picture.
    pub words: usize,
}

impl NarrationInput {
    /// Lead-in, clip and tail. Saturating because the clip is measured, not
    /// bounded: a saturated timeline is visibly absurd, a wrapped one
    /// quietly wrong.
    pub fn padded_duration_ms(&self) -> u64 {
        self.lead_in_ms
            .ms()
            .saturating_add(self.duration_ms)
            .saturating_add(self.tail_ms.ms())
    }
}

#[derive(Debug, Clone)]
pub struct ActionInput {
    pub shot_id: ShotId,
    pub scene: String,
    pub plugin: String,
    pub shot_hash: Hash,
    pub duration_ms: u64,
    pub duration_source: DurationSource,
    /// Offset from the narration's first word at which the action starts,
    /// from `cue="…"`. `None` lets the policy decide.
    pub cue_ms: Option<u64>,
    /// From `session="…"`; `None` is the scene's default session.
    pub session: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: ItemId,
    pub narration: Option<NarrationInput>,
    pub action: Option<ActionInput>,
    pub policy: PolicyKind,
    /// Where a `concurrent` action sits against its narration.
    pub align: Align,
    pub pacing: Pacing,
}

/// The part of an item's configuration the scheduler reads.
#[derive(Debug, Clone)]
pub struct Pacing {
    pub timing: TimingConfig,
    pub transition: TransitionConfig,
}

impl From<&Config> for Pacing {
    fn from(config: &Config) -> Self {
        Pacing {
            timing: config.timing.clone(),
            transition: config.transition.clone(),
        }
    }
}
