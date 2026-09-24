//! Where a duration came from. Shared vocabulary for the scheduler, the
//! timeline and the manifest, so it lives in `core` like [`VoiceSource`].
//!
//! [`VoiceSource`]: crate::VoiceSource

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DurationSource {
    Exact,
    Estimated,
    Measured,
    /// The adapter cannot say (a Playwright script). Neither an estimate nor
    /// zero: the scheduler gives the shot its line's length.
    Unknown,
}

impl DurationSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Estimated => "estimated",
            Self::Measured => "measured",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for DurationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}
