use teleprompt_core::config::TimingConfig;
use teleprompt_core::policy::Align;
use teleprompt_core::PolicyKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub narration_start_ms: u64,
    pub action_start_ms: u64,
    pub action_duration_ms: u64,
    pub item_duration_ms: u64,
    pub warnings: Vec<String>,
}

/// How an item under `policy` lays out its narration and action; `align`
/// places a `concurrent` one, and nothing else.
pub fn layout(
    policy: PolicyKind,
    align: Align,
    narration_ms: u64,
    action_ms: u64,
    timing: &TimingConfig,
) -> Layout {
    layout_at(policy, align, narration_ms, action_ms, None, timing)
}

/// [`layout`] with a cue: the offset into the item at which the action
/// starts. Only `concurrent` honours it; the compiler rejects a cue on any
/// other policy (`docs/design.md#cues`).
pub fn layout_at(
    policy: PolicyKind,
    align: Align,
    narration_ms: u64,
    action_ms: u64,
    cue_ms: Option<u64>,
    timing: &TimingConfig,
) -> Layout {
    if let (PolicyKind::Concurrent, Some(shot)) = (policy, cue_ms) {
        return Layout {
            narration_start_ms: 0,
            action_start_ms: shot,
            action_duration_ms: action_ms,
            // A late cue extends the item rather than clip the action.
            item_duration_ms: narration_ms.max(shot.saturating_add(action_ms)),
            warnings: Vec::new(),
        };
    }
    // `fit-line` has set the line's tempo already: the rest is `concurrent`
    // from the start.
    let (policy, align) = match policy {
        PolicyKind::FitLine => (PolicyKind::Concurrent, Align::Start),
        other => (other, align),
    };
    match policy {
        PolicyKind::Hold => Layout {
            narration_start_ms: 0,
            action_start_ms: narration_ms,
            action_duration_ms: action_ms,
            item_duration_ms: narration_ms.saturating_add(action_ms),
            warnings: Vec::new(),
        },

        PolicyKind::Concurrent => {
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

        PolicyKind::FitAction => {
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

        PolicyKind::FitLine => unreachable!("laid out as concurrent"),

        PolicyKind::TrimAction => {
            let mut warnings = Vec::new();
            let adjusted = if narration_ms == 0 {
                // Nothing to trim against; trimming to zero would delete
                // the action.
                action_ms
            } else if action_ms <= narration_ms {
                action_ms
            } else {
                let needed = action_ms as f64 / narration_ms as f64;
                if needed > timing.trim_warn_above {
                    warnings.push(format!(
                        "action needs {needed:.2}x speedup, above trim_warn_above {:.2}; cut to fit",
                        timing.trim_warn_above
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

/// A `fit-line` line's tempo: its clip against the `available_ms` its
/// picture leaves between lead-in and tail, within the bounds for a
/// synthesized line or a recorded take. `None` at its own pace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fit {
    pub tempo_permille: u32,
    /// The clip's length at that tempo.
    pub clip_ms: u64,
    pub warning: Option<String>,
}

pub fn fit_line(
    clip_ms: u64,
    words: usize,
    available_ms: u64,
    recorded: bool,
    timing: &TimingConfig,
) -> Option<Fit> {
    if clip_ms == 0 {
        return None;
    }
    let (min, max, which) = if recorded {
        (timing.min_take_speed, timing.max_take_speed, "take")
    } else {
        (timing.min_line_speed, timing.max_line_speed, "line")
    };
    let wanted = clip_ms as f64 / available_ms.max(1) as f64;
    let tempo = wanted.clamp(min, max);
    let tempo_permille = (tempo * 1000.0).round() as u32;
    if tempo_permille == 1000 {
        return None;
    }
    let mut fitted_ms = (clip_ms as f64 * 1000.0 / f64::from(tempo_permille)).round() as u64;
    if wanted <= max {
        // A tempo in whole thousandths can leave a fitting line a
        // millisecond over; it fits, so it ends with its picture.
        fitted_ms = fitted_ms.min(available_ms);
    }
    let warning = if wanted > max {
        let cut = (words as f64 * (1.0 - max / wanted)).ceil() as usize;
        Some(format!(
            "the line runs {clip_ms}ms against the {available_ms}ms its picture leaves: \
             {wanted:.2}x, past max_{which}_speed {max:.2}; cut about {cut} of its {words} words"
        ))
    } else if wanted < min {
        Some(format!(
            "the line runs {clip_ms}ms against the {available_ms}ms its picture leaves: \
             {wanted:.2}x, past min_{which}_speed {min:.2}; it ends {}ms early",
            available_ms.saturating_sub(fitted_ms)
        ))
    } else {
        None
    };
    Some(Fit {
        tempo_permille,
        clip_ms: fitted_ms,
        warning,
    })
}
