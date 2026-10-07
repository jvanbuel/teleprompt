//! A block's pacing policy and alignment, as an author writes them and as
//! the timeline and manifest record the policy. How they lay an item out is
//! `teleprompt_pipeline::schedule::Policy`; these live in `core` so the parser can
//! check them and the manifest can carry them without the scheduler.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyKind {
    Hold,
    Concurrent,
    FitAction,
    TrimAction,
    /// The picture leads: the line is sped up or slowed to fit it.
    FitLine,
}

impl PolicyKind {
    pub fn parse(s: &str) -> Result<Self, (String, String)> {
        match s {
            "hold" => Ok(Self::Hold),
            "concurrent" => Ok(Self::Concurrent),
            "fit-action" => Ok(Self::FitAction),
            "trim-action" => Ok(Self::TrimAction),
            "fit-line" => Ok(Self::FitLine),
            _ => Err((
                format!("unknown policy `{s}`"),
                "policy is hold|concurrent|fit-action|trim-action|fit-line".to_string(),
            )),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Concurrent => "concurrent",
            Self::FitAction => "fit-action",
            Self::TrimAction => "trim-action",
            Self::FitLine => "fit-line",
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
