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
            "stretch" => Some(Policy::Stretch),
            "trim" => Some(Policy::Trim),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Policy::Hold => "hold",
            Policy::Concurrent(_) => "concurrent",
            Policy::Stretch => "stretch",
            Policy::Trim => "trim",
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
                0
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
                beat_duration_ms: narration_ms,
                warnings,
            }
        }
    }
}
