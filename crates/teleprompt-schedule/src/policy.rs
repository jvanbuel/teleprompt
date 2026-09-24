use teleprompt_core::config::TimingConfig;
use teleprompt_core::policy::Align;
use teleprompt_core::PolicyKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Hold,
    Concurrent(Align),
    Stretch,
    Trim,
}

impl Policy {
    /// The policy an author named, aligned as they asked; `align` matters
    /// only to `concurrent`.
    pub fn new(kind: PolicyKind, align: Align) -> Self {
        match kind {
            PolicyKind::Hold => Policy::Hold,
            PolicyKind::Concurrent => Policy::Concurrent(align),
            PolicyKind::StretchAction => Policy::Stretch,
            PolicyKind::TrimAction => Policy::Trim,
        }
    }

    pub fn kind(&self) -> PolicyKind {
        match self {
            Policy::Hold => PolicyKind::Hold,
            Policy::Concurrent(_) => PolicyKind::Concurrent,
            Policy::Stretch => PolicyKind::StretchAction,
            Policy::Trim => PolicyKind::TrimAction,
        }
    }

    pub fn label(&self) -> &'static str {
        self.kind().label()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub narration_start_ms: u64,
    pub action_start_ms: u64,
    pub action_duration_ms: u64,
    pub item_duration_ms: u64,
    pub warnings: Vec<String>,
}

pub fn layout(policy: Policy, narration_ms: u64, action_ms: u64, timing: &TimingConfig) -> Layout {
    layout_at(policy, narration_ms, action_ms, None, timing)
}

/// [`layout`] with a cue: the offset into the item at which the action
/// starts. Only `concurrent` honours it; the compiler rejects a cue on any
/// other policy (`docs/design.md#cues`).
pub fn layout_at(
    policy: Policy,
    narration_ms: u64,
    action_ms: u64,
    cue_ms: Option<u64>,
    timing: &TimingConfig,
) -> Layout {
    if let (Policy::Concurrent(_), Some(shot)) = (policy, cue_ms) {
        return Layout {
            narration_start_ms: 0,
            action_start_ms: shot,
            action_duration_ms: action_ms,
            // A late cue extends the item rather than clip the action.
            item_duration_ms: narration_ms.max(shot.saturating_add(action_ms)),
            warnings: Vec::new(),
        };
    }
    match policy {
        Policy::Hold => Layout {
            narration_start_ms: 0,
            action_start_ms: narration_ms,
            action_duration_ms: action_ms,
            item_duration_ms: narration_ms.saturating_add(action_ms),
            warnings: Vec::new(),
        },

        Policy::Concurrent(align) => {
            let item = narration_ms.max(action_ms);
            let (n_start, a_start) = match align {
                Align::Start => (0, 0),
                Align::End => (item - narration_ms, item - action_ms),
                Align::Center => ((item - narration_ms) / 2, (item - action_ms) / 2),
            };
            Layout {
                narration_start_ms: n_start,
                action_start_ms: a_start,
                action_duration_ms: action_ms,
                item_duration_ms: item,
                warnings: Vec::new(),
            }
        }

        Policy::Stretch => {
            let mut warnings = Vec::new();
            let adjusted = if action_ms == 0 {
                0
            } else if narration_ms == 0 {
                // A shot after a mark has no narration of its own, so
                // there is nothing to fill: keep its length rather than
                // clamp a zero factor to `min_stretch`.
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
                item_duration_ms: narration_ms.max(adjusted),
                warnings,
            }
        }

        Policy::Trim => {
            let mut warnings = Vec::new();
            let adjusted = if narration_ms == 0 {
                // Nothing to trim against; trimming to zero would delete
                // the action.
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
                item_duration_ms: narration_ms.max(adjusted),
                warnings,
            }
        }
    }
}
