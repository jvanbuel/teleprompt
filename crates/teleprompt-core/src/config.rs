//! The words configuration is spoken in, which crates that never read a
//! project file share: timing, the video's frame, transitions and a scene's
//! settings. The project file itself, `teleprompt.toml` and its layers, is
//! `teleprompt_script::config`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::DurationMs;

#[derive(Debug, Clone, PartialEq)]
pub struct TimingConfig {
    pub lead_in_ms: DurationMs,
    pub tail_ms: DurationMs,
    /// Added to a line's lead-in when its speaker is not the last line's:
    /// the pause before someone answers.
    pub turn_gap_ms: DurationMs,
    pub max_stretch: f64,
    pub min_stretch: f64,
    pub trim_warn_above: f64,
    /// How much faster or slower `fit-line` may play a synthesized line,
    /// and a recorded take (docs/design.md#led-by-the-picture).
    pub min_line_speed: f64,
    pub max_line_speed: f64,
    pub min_take_speed: f64,
    pub max_take_speed: f64,
    /// The whole video's length, which `check` warns past.
    pub length_ms: Option<DurationMs>,
}

/// The shape of the rendered video.
///
/// Separate from [`TransitionConfig`] even though front matter nests both
/// under `output:`, because the two are read by different things: a
/// transition is scheduling, which every command sees, and this is the
/// frame, which only a render has any use for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputConfig {
    pub resolution: (u32, u32),
    pub fps: u32,
    /// Whether each speaker is named on screen when they first speak.
    pub names: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionDuration {
    Auto,
    Fixed(DurationMs),
}

/// Accepts `auto` (case-insensitively) or any string `DurationMs::parse`
/// understands (e.g. `250ms`, `1s`). Anything else is a deserialize error
/// naming the offending value, so a typo in front matter is reported rather
/// than silently treated as `auto`.
impl<'de> Deserialize<'de> for TransitionDuration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s.eq_ignore_ascii_case("auto") {
            return Ok(TransitionDuration::Auto);
        }
        DurationMs::parse(&s)
            .map(TransitionDuration::Fixed)
            .map_err(|_| {
                serde::de::Error::custom(format!(
                    "`{s}` is not a valid transition duration \
                     (expected `auto` or a duration like `250ms`)"
                ))
            })
    }
}

/// How one shot gives way to the next. The kinds the renderer draws have a
/// variant each; the vocabulary stays open, and any other name is kept as
/// written in [`OtherKind`], which the renderer draws as a fade.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TransitionKind {
    Crossfade,
    Dissolve,
    Wipe,
    /// No blend: what a shot that nothing follows gets.
    Cut,
    Other(OtherKind),
}

/// A transition name teleprompt does not know. Only [`TransitionKind::parse`]
/// makes one, so it is never a known name in disguise.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OtherKind(String);

impl OtherKind {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TransitionKind {
    pub fn parse(name: &str) -> Self {
        match name {
            "crossfade" => Self::Crossfade,
            "dissolve" => Self::Dissolve,
            "wipe" => Self::Wipe,
            "cut" => Self::Cut,
            other => Self::Other(OtherKind(other.to_string())),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Crossfade => "crossfade",
            Self::Dissolve => "dissolve",
            Self::Wipe => "wipe",
            Self::Cut => "cut",
            Self::Other(other) => other.as_str(),
        }
    }
}

impl std::fmt::Display for TransitionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for TransitionKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TransitionKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(d)?))
    }
}

/// How one shot gives way to the next, as scheduled: what the timeline,
/// the manifest and the renderer all carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration_ms: crate::SpanMs,
}

impl Transition {
    /// A hard cut, which is what a shot that nothing follows also gets.
    pub fn cut() -> Self {
        Self {
            kind: TransitionKind::Cut,
            duration_ms: crate::SpanMs::ZERO,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionConfig {
    pub kind: TransitionKind,
    pub duration: TransitionDuration,
    pub min_ms: DurationMs,
    pub max_ms: DurationMs,
}

/// A resolved scene's scene plugin and its scene plugin-native settings.
///
/// `settings` holds `serde_json::Value`, not `String`: a scene may configure
/// `browser.viewport: [1920, 1080]`, and flattening structured YAML into a
/// string map either rejects it (which is what happened) or lossily stringifies
/// it. Scene plugins read whatever shape their own tool wants.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneConfig {
    /// The scene plugin that records it.
    pub plugin: String,
    pub settings: BTreeMap<String, serde_json::Value>,
    /// What a relative path in `settings` is relative to: the project's
    /// directory, set by whoever knows it. Not part of any key, so a cache
    /// moves between machines.
    pub root: std::path::PathBuf,
}

impl SceneConfig {
    /// A stable string for the settings, for hashing into a key.
    ///
    /// Here rather than at the call site because this is the type that
    /// knows what its settings are: JSON of a `BTreeMap` is ordered and
    /// round-trips, where `{:?}` is a debug format nothing promises to keep,
    /// and it does not depend on which YAML library read the settings.
    /// The setting `key` as a path, or `default`, against [`Self::root`].
    pub fn path(&self, key: &str, default: &str) -> std::path::PathBuf {
        let given = self.settings.get(key).and_then(|v| v.as_str());
        self.root.join(given.unwrap_or(default))
    }

    pub fn settings_fingerprint(&self) -> String {
        // No settings is nothing to hash, declared or not.
        if self.settings.is_empty() {
            return String::new();
        }
        serde_json::to_string(&self.settings).unwrap_or_default()
    }
}
