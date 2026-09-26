//! A block's pacing policy and alignment, as an author writes them and as
//! the timeline and manifest record the policy. How they lay an item out is
//! `teleprompt_schedule::Policy`; these live in `core` so the parser can
//! check them and the manifest can carry them without the scheduler.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyKind {
    Hold,
    Concurrent,
    FitAction,
    TrimAction,
}

impl PolicyKind {
    /// An old spelling is an error, not an alias, and names its replacement
    /// (`docs/design.md#policies`).
    pub fn parse(s: &str) -> Result<Self, (String, String)> {
        match s {
            "hold" => Ok(Self::Hold),
            "concurrent" => Ok(Self::Concurrent),
            "fit-action" => Ok(Self::FitAction),
            "trim-action" => Ok(Self::TrimAction),
            "stretch" | "stretch-action" | "trim" => {
                let current = if s == "trim" {
                    "trim-action"
                } else {
                    "fit-action"
                };
                Err((
                    format!("policy `{s}` was renamed to `{current}`"),
                    format!("write `policy={current}`; it adjusts the action, never the narration"),
                ))
            }
            _ => Err((
                format!("unknown policy `{s}`"),
                "policy is hold|concurrent|fit-action|trim-action".to_string(),
            )),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Concurrent => "concurrent",
            Self::FitAction => "fit-action",
            Self::TrimAction => "trim-action",
        }
    }
}

impl std::fmt::Display for PolicyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Where a `concurrent` action sits against its narration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    End,
    Center,
}

impl Align {
    pub fn parse(s: &str) -> Result<Self, (String, String)> {
        match s {
            "start" => Ok(Self::Start),
            "end" => Ok(Self::End),
            "center" => Ok(Self::Center),
            _ => Err((
                format!("unknown align `{s}`"),
                "align is start|end|center".to_string(),
            )),
        }
    }
}
