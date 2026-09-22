use teleprompt_core::config::TimingConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    End,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Hold,
    Concurrent(Align),
    Stretch,
    Trim,
}

impl Policy {
    pub fn parse(policy: &str, align: &str) -> Option<Self> {
        let align = match align {
            "start" => Align::Start,
            "end" => Align::End,
            "center" => Align::Center,
            _ => return None,
        };
        match policy {
            "hold" => Some(Policy::Hold),
            "concurrent" => Some(Policy::Concurrent(align)),
            "stretch-action" => Some(Policy::Stretch),
            "trim-action" => Some(Policy::Trim),
            _ => None,
        }
    }

    /// The current spelling of a policy that used to be called something else.
    ///
    /// `stretch` and `trim` named an operation without naming its object, so
    /// both read as though the *speech* were being adjusted — the first
    /// question anyone asked of this design was whether teleprompt
    /// time-stretches a voice. It does not: these policies only ever change
    /// how long the action takes. The old spellings are rejected rather than
    /// aliased, so the corpus converges on one name, but an author who writes
    /// an old one is told what to write instead of getting a bare "unknown
    /// policy".
    pub fn renamed_hint(policy: &str) -> Option<&'static str> {
        match policy {
            "stretch" => Some("stretch-action"),
            "trim" => Some("trim-action"),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Policy::Hold => "hold",
            Policy::Concurrent(_) => "concurrent",
            Policy::Stretch => "stretch-action",
            Policy::Trim => "trim-action",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub narration_start_ms: u64,
    pub action_start_ms: u64,
    pub action_duration_ms: u64,
    pub beat_duration_ms: u64,
    pub warnings: Vec<String>,
}

pub fn layout(policy: Policy, narration_ms: u64, action_ms: u64, timing: &TimingConfig) -> Layout {
    layout_at(policy, narration_ms, action_ms, None, timing)
}

/// [`layout`] with a cue: the offset into the narration at which the action
/// should start.
///
/// A cue is how an author says "type the command while the voice is saying
/// it". Without one, a concurrent action starts with the paragraph — which
/// is right when the paragraph is about the action from its first word, and
/// wrong the moment the sentence that names the command is the third one.
///
/// It only applies where the two run together. `hold` puts the action after
/// the narration and the stretch policies size it to the narration, so a
/// cue there would contradict the policy rather than refine it; the caller
/// rejects that pairing before this is reached.
pub fn layout_at(
    policy: Policy,
    narration_ms: u64,
    action_ms: u64,
    at_ms: Option<u64>,
    timing: &TimingConfig,
) -> Layout {
    if let (Policy::Concurrent(_), Some(cue)) = (policy, at_ms) {
        return Layout {
            narration_start_ms: 0,
            action_start_ms: cue,
            action_duration_ms: action_ms,
            // An action cued late enough to outlast the sentence extends
            // the beat; clipping it would drop the end of the very thing
            // the cue exists to show.
            beat_duration_ms: narration_ms.max(cue.saturating_add(action_ms)),
            warnings: Vec::new(),
        };
    }
    match policy {
        Policy::Hold => Layout {
            narration_start_ms: 0,
            action_start_ms: narration_ms,
            action_duration_ms: action_ms,
            beat_duration_ms: narration_ms + action_ms,
            warnings: Vec::new(),
        },

        Policy::Concurrent(align) => {
            let beat = narration_ms.max(action_ms);
            let (n_start, a_start) = match align {
                Align::Start => (0, 0),
                Align::End => (beat - narration_ms, beat - action_ms),
                Align::Center => ((beat - narration_ms) / 2, (beat - action_ms) / 2),
            };
            Layout {
                narration_start_ms: n_start,
                action_start_ms: a_start,
                action_duration_ms: action_ms,
                beat_duration_ms: beat,
                warnings: Vec::new(),
            }
        }

        Policy::Stretch => {
            let mut warnings = Vec::new();
            let adjusted = if action_ms == 0 {
                0
            } else if narration_ms == 0 {
                // A cue after a mark has no narration of its own — the
                // paragraph belongs to the block's first cue, and the rest
                // run under whatever the policy left of it. There is
                // nothing here to fill, so the tape keeps its own length.
                // Falling through would compute a factor of zero and clamp
                // it to `min_stretch`, which is the scheduler rewriting a
                // tape it was never asked about.
                action_ms
            } else {
                let wanted = narration_ms as f64 / action_ms as f64;
                let factor = if wanted > timing.max_stretch {
                    warnings.push(format!(
                        "action needs {wanted:.2}x stretch to fill narration, above max_stretch {:.2}; clamped",
                        timing.max_stretch
                    ));
                    timing.max_stretch
                } else if wanted < timing.min_stretch {
                    warnings.push(format!(
                        "action needs {wanted:.2}x stretch to fit narration, below min_stretch {:.2}; clamped",
                        timing.min_stretch
                    ));
                    timing.min_stretch
                } else {
                    wanted
                };
                (action_ms as f64 * factor).round() as u64
            };
            Layout {
                narration_start_ms: 0,
                action_start_ms: 0,
                action_duration_ms: adjusted,
                beat_duration_ms: narration_ms.max(adjusted),
                warnings,
            }
        }

        Policy::Trim => {
            let mut warnings = Vec::new();
            let adjusted = if narration_ms == 0 {
                // Nothing to trim against, for the same reason as above.
                // Trimming to zero deleted the action from the video.
                action_ms
            } else if action_ms <= narration_ms {
                action_ms
            } else {
                let needed = action_ms as f64 / narration_ms as f64;
                if needed > timing.max_speedup {
                    warnings.push(format!(
                        "action needs {needed:.2}x speedup, above max_speedup {:.2}; cut to fit",
                        timing.max_speedup
                    ));
                }
                narration_ms
            };
            Layout {
                narration_start_ms: 0,
                action_start_ms: 0,
                action_duration_ms: adjusted,
                // The beat is as long as whichever of the two is left:
                // normally the narration, and the action where there is no
                // narration to trim against.
                beat_duration_ms: narration_ms.max(adjusted),
                warnings,
            }
        }
    }
}
