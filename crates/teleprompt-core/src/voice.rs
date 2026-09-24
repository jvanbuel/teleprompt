//! Which tier of the dubbing spectrum produced (or should produce) a
//! line's audio. Shared vocabulary lives in `core` so `schedule` needs no
//! `voice` dependency (docs/design.md#crates); the ladder that walks the
//! tiers, `resolve_source`, is in `teleprompt-voice`.

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

/// `text` with every mapped word replaced by how it is said, for synthesis
/// only (docs/design.md#word-timings). Whole words, case-sensitively:
/// rewriting the inside of a longer word produces something nobody wrote.
/// Punctuation is not part of a word, so `(MWAA)` and `MWAA.` still match.
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
