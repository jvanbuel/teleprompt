//! Which tier of the dubbing spectrum produced (or should produce) a
//! segment's audio.
//!
//! This lives in `core` rather than in `teleprompt-voice` because it is
//! shared *vocabulary*, not backend machinery: `teleprompt-schedule` records
//! it on every narration input and every timeline entry, and the plan's whole
//! reason for creating `teleprompt-compile` was that `core`, `scene`,
//! `voice`, and `schedule` must not depend on one another. Keeping the enum
//! next to `Hash`, `Config`, and `SourceSpan` — the other types every crate
//! names — is what lets `schedule` stay free of a `voice` dependency.
//!
//! The ladder itself (`resolve_source`) stays in `teleprompt-voice`: walking
//! it needs to ask backends what they can do, which is squarely that crate's
//! business.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceSource {
    Recorded,
    Cloned,
    Synthetic,
}

impl VoiceSource {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "recorded" => Some(Self::Recorded),
            "cloned" => Some(Self::Cloned),
            "synthetic" => Some(Self::Synthetic),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::Cloned => "cloned",
            Self::Synthetic => "synthetic",
        }
    }

    pub fn next_lower(&self) -> Option<Self> {
        match self {
            Self::Recorded => Some(Self::Cloned),
            Self::Cloned => Some(Self::Synthetic),
            Self::Synthetic => None,
        }
    }
}
