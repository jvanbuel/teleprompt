use std::collections::BTreeMap;

use serde::Deserialize;

use crate::attrs::{parse_duration_ms, Attributes};

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub locales: Locales,
    pub voice: VoiceConfig,
    pub timing: TimingConfig,
    pub transition: TransitionConfig,
    pub scenes: BTreeMap<String, SceneConfig>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Locales {
    pub source: String,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceConfig {
    pub source: String,
    pub backend: String,
    pub voice: Option<String>,
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimingConfig {
    pub lead_in_ms: u64,
    pub tail_ms: u64,
    pub max_stretch: f64,
    pub min_stretch: f64,
    pub max_speedup: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionDuration {
    Auto,
    Fixed(u64),
}

/// Accepts `auto` (case-insensitively) or any string `parse_duration_ms`
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
        parse_duration_ms(&s)
            .map(TransitionDuration::Fixed)
            .map_err(|_| {
                serde::de::Error::custom(format!(
                    "`{s}` is not a valid transition duration \
                     (expected `auto` or a duration like `250ms`)"
                ))
            })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionConfig {
    pub kind: String,
    pub duration: TransitionDuration,
    pub min_ms: u64,
    pub max_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneConfig {
    pub adapter: String,
    pub settings: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            locales: Locales {
                source: "en".into(),
                targets: Vec::new(),
            },
            voice: VoiceConfig {
                source: "synthetic".into(),
                backend: "null".into(),
                voice: None,
                speed: 1.0,
            },
            timing: TimingConfig {
                lead_in_ms: 150,
                tail_ms: 150,
                max_stretch: 3.0,
                min_stretch: 0.33,
                max_speedup: 2.0,
            },
            transition: TransitionConfig {
                kind: "crossfade".into(),
                duration: TransitionDuration::Auto,
                min_ms: 0,
                max_ms: 600,
            },
            scenes: BTreeMap::new(),
        }
    }
}

/// Default adapter for a scene name, used when config omits it.
pub fn default_adapter(scene: &str) -> &'static str {
    match scene {
        "browser" => "playwright",
        "terminal" => "vhs",
        "media" => "media",
        _ => "mock",
    }
}

/// All-optional mirror of `Config`, deserialized from one configuration layer
/// (`teleprompt.toml`, script/chapter front matter, or segment/block attributes).
///
/// Carries `teleprompt` (schema version) and `output` (resolution/fps/transition)
/// so real front matter — which nests `transition` under `output:` and stamps a
/// top-level `teleprompt: 1` — deserializes. `teleprompt`, `output.resolution`,
/// and `output.fps` are parsed and otherwise unused until M1.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialConfig {
    pub teleprompt: Option<u32>,
    pub locales: Option<PartialLocales>,
    pub voice: Option<PartialVoice>,
    pub timing: Option<PartialTiming>,
    pub output: Option<PartialOutput>,
    pub scene: Option<BTreeMap<String, PartialScene>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialLocales {
    pub source: Option<String>,
    pub targets: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialVoice {
    pub source: Option<String>,
    pub backend: Option<String>,
    pub voice: Option<String>,
    pub speed: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialTiming {
    pub lead_in_ms: Option<u64>,
    pub tail_ms: Option<u64>,
    pub max_stretch: Option<f64>,
    pub min_stretch: Option<f64>,
    pub max_speedup: Option<f64>,
}

/// The `output:` front-matter block (spec §3.1). `resolution` and `fps` are
/// declared so real front matter deserializes; they are unused until M1.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialOutput {
    pub resolution: Option<Vec<u32>>,
    pub fps: Option<u32>,
    pub transition: Option<PartialTransition>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialTransition {
    pub kind: Option<String>,
    pub duration: Option<TransitionDuration>,
    pub min_ms: Option<u64>,
    pub max_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PartialScene {
    pub adapter: Option<String>,
    #[serde(flatten)]
    pub settings: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("invalid TOML: {0}")]
    Toml(#[from] toml::de::Error),
}

impl PartialConfig {
    pub fn from_yaml(s: &str) -> Result<Self, ConfigError> {
        if s.trim().is_empty() {
            return Ok(Self::default());
        }
        Ok(serde_yaml::from_str(s)?)
    }

    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(s)?)
    }

    /// Builds a layer from segment or block attributes. Unparseable values are
    /// dropped here; `parse_attrs` has already reported them as diagnostics.
    pub fn from_attrs(a: &Attributes) -> Self {
        let mut c = Self::default();
        let mut timing = PartialTiming::default();
        let mut voice = PartialVoice::default();

        if let Some(Ok(ms)) = a.get_ms("lead_in") {
            timing.lead_in_ms = Some(ms);
        }
        if let Some(Ok(ms)) = a.get_ms("tail") {
            timing.tail_ms = Some(ms);
        }
        if let Some(Ok(v)) = a.get_f64("max_stretch") {
            timing.max_stretch = Some(v);
        }
        if let Some(Ok(v)) = a.get_f64("min_stretch") {
            timing.min_stretch = Some(v);
        }
        if let Some(Ok(v)) = a.get_f64("max_speedup") {
            timing.max_speedup = Some(v);
        }
        voice.source = a.get("voice.source").map(str::to_string);
        voice.backend = a.get("voice.backend").map(str::to_string);
        voice.voice = a.get("voice.voice").map(str::to_string);
        if let Some(Ok(v)) = a.get_f64("voice.speed") {
            voice.speed = Some(v);
        }

        c.timing = Some(timing);
        c.voice = Some(voice);
        c
    }
}

macro_rules! set {
    ($target:expr, $src:expr) => {
        if let Some(v) = $src {
            $target = v;
        }
    };
}

impl Config {
    pub fn merged(layers: &[PartialConfig]) -> Self {
        let mut c = Config::default();
        for layer in layers {
            if let Some(l) = &layer.locales {
                set!(c.locales.source, l.source.clone());
                set!(c.locales.targets, l.targets.clone());
            }
            if let Some(v) = &layer.voice {
                set!(c.voice.source, v.source.clone());
                set!(c.voice.backend, v.backend.clone());
                if v.voice.is_some() {
                    c.voice.voice = v.voice.clone();
                }
                set!(c.voice.speed, v.speed);
            }
            if let Some(t) = &layer.timing {
                set!(c.timing.lead_in_ms, t.lead_in_ms);
                set!(c.timing.tail_ms, t.tail_ms);
                set!(c.timing.max_stretch, t.max_stretch);
                set!(c.timing.min_stretch, t.min_stretch);
                set!(c.timing.max_speedup, t.max_speedup);
            }
            if let Some(t) = layer.output.as_ref().and_then(|o| o.transition.as_ref()) {
                set!(c.transition.kind, t.kind.clone());
                set!(c.transition.min_ms, t.min_ms);
                set!(c.transition.max_ms, t.max_ms);
                set!(c.transition.duration, t.duration.clone());
            }
            if let Some(scenes) = &layer.scene {
                for (name, ps) in scenes {
                    let entry = c.scenes.entry(name.clone()).or_insert_with(|| SceneConfig {
                        adapter: default_adapter(name).to_string(),
                        settings: BTreeMap::new(),
                    });
                    set!(entry.adapter, ps.adapter.clone());
                    for (k, v) in &ps.settings {
                        entry.settings.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        c
    }
}
