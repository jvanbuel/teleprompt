//! Which tier of the dubbing spectrum produced (or should produce) a
//! line's audio. Shared vocabulary lives in `core` so `schedule` needs no
//! `voice` dependency (docs/design.md#crates); the ladder that walks the
//! tiers, `resolve_source`, is in `teleprompt-voice`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
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

    /// Whether `other` is somewhere down the ladder from this tier.
    pub fn is_above(self, other: Self) -> bool {
        let mut tier = self.next_lower();
        while let Some(t) = tier {
            if t == other {
                return true;
            }
            tier = t.next_lower();
        }
        false
    }

    pub fn next_lower(&self) -> Option<Self> {
        match self {
            Self::Recorded => Some(Self::Cloned),
            Self::Cloned => Some(Self::Synthetic),
            Self::Synthetic => None,
        }
    }
}

/// The tier a line asked for and, when the ladder could not deliver it,
/// what it got instead and why. A reason with no downgrade, a downgrade
/// with no reason, or a "downgrade" to a higher tier cannot be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceTier {
    requested: VoiceSource,
    downgrade: Option<Downgrade>,
}

/// The lower tier a line was delivered at, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downgrade {
    pub to: VoiceSource,
    pub reason: String,
}

impl VoiceTier {
    pub fn delivered(requested: VoiceSource) -> Self {
        Self {
            requested,
            downgrade: None,
        }
    }

    /// `to` must be a lower tier than `requested`.
    pub fn downgraded(
        requested: VoiceSource,
        to: VoiceSource,
        reason: String,
    ) -> Result<Self, String> {
        if !requested.is_above(to) {
            return Err(format!(
                "`{to}` is not a lower tier than `{requested}`, so it is not a downgrade"
            ));
        }
        Ok(Self {
            requested,
            downgrade: Some(Downgrade { to, reason }),
        })
    }

    /// From the three fields the timeline and manifest publish, refusing any
    /// combination that contradicts itself.
    pub fn from_fields(
        requested: VoiceSource,
        actual: VoiceSource,
        reason: Option<String>,
    ) -> Result<Self, String> {
        match reason {
            None if actual == requested => Ok(Self::delivered(requested)),
            None => Err(format!(
                "`{requested}` was delivered as `{actual}` with no reason given"
            )),
            Some(_) if actual == requested => Err(format!(
                "a downgrade reason is given, but `{requested}` was delivered"
            )),
            Some(reason) => Self::downgraded(requested, actual, reason),
        }
    }

    pub fn requested(&self) -> VoiceSource {
        self.requested
    }

    pub fn actual(&self) -> VoiceSource {
        self.downgrade.as_ref().map_or(self.requested, |d| d.to)
    }

    pub fn downgrade(&self) -> Option<&Downgrade> {
        self.downgrade.as_ref()
    }

    pub fn reason(&self) -> Option<&str> {
        self.downgrade.as_ref().map(|d| d.reason.as_str())
    }
}

impl std::fmt::Display for VoiceSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
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
