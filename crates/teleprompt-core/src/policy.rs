//! The name of a pacing policy, as the timeline and manifest record it.
//! The policy itself, with its alignment and layout, is
//! `teleprompt_schedule::Policy`; the name lives in `core` so the
//! manifest can carry it without depending on the scheduler.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyKind {
    Hold,
    Concurrent,
    StretchAction,
    TrimAction,
}

impl PolicyKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Concurrent => "concurrent",
            Self::StretchAction => "stretch-action",
            Self::TrimAction => "trim-action",
        }
    }
}

impl std::fmt::Display for PolicyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}
