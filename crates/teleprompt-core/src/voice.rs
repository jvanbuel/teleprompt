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

/// `text` with every mapped word replaced by how it is said.
///
/// A synthetic voice reads what it is given, and what it is given is
/// spelling. `MWAA` comes out as a word, product names come out as
/// whatever their letters suggest, and no amount of re-recording fixes it
/// because the author never wrote anything wrong. The map is the place to
/// say "this is how that is pronounced", and it applies to synthesis only:
/// captions, the manifest and the script keep the spelling, because that
/// is what a reader wants to see.
///
/// Whole words, case-sensitively. A map entry names a word, not a
/// substring: rewriting the inside of a longer word produces something
/// nobody wrote. Punctuation is not part of the word, so an acronym in
/// brackets or before a full stop is still found — which is most of the
/// places an acronym appears.
pub fn spoken(text: &str, pronounce: &std::collections::BTreeMap<String, String>) -> String {
    if pronounce.is_empty() {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    for ch in text.chars() {
        // An apostrophe belongs to the word it sits in (`MWAA's`), but the
        // possessive is not part of the word being looked up, so it is not
        // collected either — it lands in `out` and the word before it is
        // resolved on its own.
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            word.push(ch);
            continue;
        }
        flush_word(&mut out, &mut word, pronounce);
        out.push(ch);
    }
    flush_word(&mut out, &mut word, pronounce);
    out
}

fn flush_word(
    out: &mut String,
    word: &mut String,
    pronounce: &std::collections::BTreeMap<String, String>,
) {
    if word.is_empty() {
        return;
    }
    match pronounce.get(word.as_str()) {
        Some(said) => out.push_str(said),
        None => out.push_str(word),
    }
    word.clear();
}
