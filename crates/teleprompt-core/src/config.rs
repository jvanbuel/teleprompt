use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::attrs::{BlockAttrs, LineAttrs};
use crate::DurationMs;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub locales: Locales,
    pub voice: VoiceConfig,
    pub timing: TimingConfig,
    pub output: OutputConfig,
    pub transition: TransitionConfig,
    pub scenes: BTreeMap<String, SceneConfig>,
    /// `scene.default:` from front matter. Resolved through the layer merge
    /// like everything else, but not yet consulted: every action block names
    /// its own `scene=`. Kept here rather than discarded so the value an author
    /// wrote survives the merge instead of being silently lost.
    pub default_scene: Option<String>,
    /// Backend-native settings, keyed by backend id, exactly as written
    /// under `backends:` in config or front matter.
    ///
    /// Core never interprets these. It cannot: the whole point of the
    /// pluggable contract is that teleprompt does not know what backends
    /// exist, so a named field per backend here would be a hardcoded list
    /// wearing a config's clothes. Each backend deserializes its own slice
    /// by its own id, and an id this build does not ship is not an error at
    /// this layer — `voice.backend` selection reports that, with the
    /// available list.
    pub backends: BTreeMap<String, serde_yaml::Value>,
    pub translate: TranslateConfig,
}

/// What `translate` translates with. The provider's own settings (where
/// to reach it, a key) are under `backends.<provider>`, as a voice's are.
#[derive(Debug, Clone, PartialEq)]
pub struct TranslateConfig {
    pub provider: String,
    /// The provider's model; its own default when unset.
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Locales {
    pub source: String,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceConfig {
    pub backend: String,
    pub voice: Option<String>,
    pub speed: f64,
    /// How to deliver each line, for a backend that takes instructions.
    pub instruct: Option<String>,
    /// How to say words a voice gets wrong, applied to synthesis only.
    /// The script, the captions and the manifest keep the spelling.
    pub pronounce: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimingConfig {
    pub lead_in_ms: DurationMs,
    pub tail_ms: DurationMs,
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

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionConfig {
    pub kind: TransitionKind,
    pub duration: TransitionDuration,
    pub min_ms: DurationMs,
    pub max_ms: DurationMs,
}

/// A resolved scene's adapter and its adapter-native settings.
///
/// `settings` holds `serde_yaml::Value`, not `String`: a scene may configure
/// `browser.viewport: [1920, 1080]`, and flattening structured YAML into a
/// string map either rejects it (which is what happened) or lossily stringifies
/// it. Adapters read whatever shape their own tool wants.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneConfig {
    pub adapter: String,
    pub settings: BTreeMap<String, serde_yaml::Value>,
}

impl SceneConfig {
    /// A stable string for the settings, for hashing into a key.
    ///
    /// Here rather than at the call site because this is the type that
    /// knows what its settings are: YAML of a `BTreeMap` is ordered and
    /// round-trips, where `{:?}` is a debug format nothing promises to keep.
    pub fn settings_fingerprint(&self) -> String {
        // No settings is nothing to hash, declared or not.
        if self.settings.is_empty() {
            return String::new();
        }
        serde_yaml::to_string(&self.settings).unwrap_or_default()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            locales: Locales {
                source: "en".into(),
                targets: Vec::new(),
            },
            voice: VoiceConfig {
                backend: "null".into(),
                voice: None,
                speed: 1.0,
                instruct: None,
                pronounce: BTreeMap::new(),
            },
            output: OutputConfig {
                resolution: (1920, 1080),
                fps: 30,
            },
            timing: TimingConfig {
                lead_in_ms: DurationMs::millis(150),
                tail_ms: DurationMs::millis(150),
                max_stretch: 3.0,
                min_stretch: 0.33,
                trim_warn_above: 2.0,
                min_line_speed: 0.9,
                max_line_speed: 1.15,
                min_take_speed: 0.95,
                max_take_speed: 1.08,
                length_ms: None,
            },
            transition: TransitionConfig {
                kind: TransitionKind::Crossfade,
                duration: TransitionDuration::Auto,
                min_ms: DurationMs::ZERO,
                max_ms: DurationMs::millis(600),
            },
            scenes: BTreeMap::new(),
            default_scene: None,
            backends: BTreeMap::new(),
            // Local, so a script is translated on the machine by default.
            translate: TranslateConfig {
                provider: "ollama".into(),
                model: None,
            },
        }
    }
}

/// The adapter a scene uses when none is declared for it: the adapter of
/// the same name.
pub fn default_adapter(scene: &str) -> &str {
    match scene {
        // Older names, from before an adapter's name was a scene of its own.
        "browser" => "playwright",
        "terminal" => "vhs",
        name => name,
    }
}

/// All-optional mirror of `Config`, deserialized from one configuration layer
/// (`teleprompt.toml`, script/chapter front matter, or line/block attributes).
///
/// Carries `teleprompt` (schema version) and `output`
/// (resolution/fps/transition) so real front matter — which nests `transition`
/// under `output:` and stamps a top-level `teleprompt: 1` — deserializes.
/// `teleprompt` is parsed and otherwise unused.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialConfig {
    pub teleprompt: Option<u32>,
    pub locales: Option<PartialLocales>,
    pub voice: Option<PartialVoice>,
    pub timing: Option<PartialTiming>,
    pub output: Option<PartialOutput>,
    pub scene: Option<PartialScenes>,
    pub backends: Option<BTreeMap<String, serde_yaml::Value>>,
    /// `[locale.nl]`: settings for compiling in one locale, a Dutch voice
    /// say, applied over the rest of the layer they are written in.
    pub locale: Option<BTreeMap<String, PartialConfig>>,
    pub translate: Option<PartialTranslate>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialTranslate {
    pub provider: Option<String>,
    pub model: Option<String>,
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
    pub backend: Option<String>,
    pub voice: Option<String>,
    pub speed: Option<f64>,
    pub instruct: Option<String>,
    /// `pronounce: { MWAA: em-double-you-ay-ay }`. Merged across layers
    /// word by word rather than replaced wholesale: a script adding one
    /// name should not drop the project's list.
    pub pronounce: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialTiming {
    pub lead_in_ms: Option<DurationMs>,
    pub tail_ms: Option<DurationMs>,
    pub max_stretch: Option<f64>,
    pub min_stretch: Option<f64>,
    pub trim_warn_above: Option<f64>,
    pub min_line_speed: Option<f64>,
    pub max_line_speed: Option<f64>,
    pub min_take_speed: Option<f64>,
    pub max_take_speed: Option<f64>,
    pub length_ms: Option<DurationMs>,
    /// Renamed to `trim_warn_above`. Read only to say so: as an unknown
    /// field it would fail without naming the new key.
    #[serde(default, deserialize_with = "renamed_max_speedup")]
    pub max_speedup: Option<Renamed>,
}

/// A config key that was renamed; it never deserializes.
#[derive(Debug, Clone, Copy)]
pub enum Renamed {}

fn renamed_max_speedup<'de, D: serde::Deserializer<'de>>(
    _: D,
) -> Result<Option<Renamed>, D::Error> {
    Err(serde::de::Error::custom(
        "`timing.max_speedup` was renamed to `timing.trim_warn_above`: \
         it sets when trimming is reported, and never sped anything up",
    ))
}

/// The `output:` front-matter block: frame size, rate and transitions.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialOutput {
    pub resolution: Option<Resolution>,
    pub fps: Option<u32>,
    pub transition: Option<PartialTransition>,
}

/// `resolution: [1920, 1080]`, as front matter writes it.
///
/// A dedicated type rather than `Vec<u32>` so that a list of any other
/// length is a deserialize error naming the field, the way a malformed
/// transition duration is. Merging would otherwise have to choose between
/// ignoring `[1920]` — an author rendering at the wrong size and finding
/// out after the wait — and inventing a missing half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution(pub u32, pub u32);

impl<'de> Deserialize<'de> for Resolution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let parts = Vec::<u32>::deserialize(deserializer)?;
        match parts[..] {
            [w, h] => Ok(Resolution(w, h)),
            _ => Err(serde::de::Error::custom(format!(
                "`output.resolution` takes a width and a height \
                 (e.g. `[1920, 1080]`), but has {} value(s)",
                parts.len()
            ))),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialTransition {
    pub kind: Option<TransitionKind>,
    pub duration: Option<TransitionDuration>,
    pub min_ms: Option<DurationMs>,
    pub max_ms: Option<DurationMs>,
}

/// The `scene:` block, which takes two shapes at once:
///
/// ```yaml
/// scene:
///   default: browser            # names a scene
///   browser:                    # configures one
///     base_url: "http://localhost:3000"
/// ```
///
/// `default` is read first as a name. An untagged "name or settings" enum
/// would also accept `scene: { browser: playwright }`, turning a typo into a
/// silent no-op.
#[derive(Debug, Clone, Default)]
pub struct PartialScenes {
    pub default: Option<String>,
    pub scenes: BTreeMap<String, PartialScene>,
}

impl<'de> Deserialize<'de> for PartialScenes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct MapVisitor;

        impl<'de> serde::de::Visitor<'de> for MapVisitor {
            type Value = PartialScenes;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a `scene` block: `default: <name>` and/or per-scene settings")
            }

            fn visit_map<A>(self, mut map: A) -> Result<PartialScenes, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut out = PartialScenes::default();
                while let Some(key) = map.next_key::<String>()? {
                    if key == "default" {
                        out.default = Some(map.next_value::<String>()?);
                    } else {
                        out.scenes.insert(key, map.next_value::<PartialScene>()?);
                    }
                }
                Ok(out)
            }
        }

        deserializer.deserialize_map(MapVisitor)
    }
}

/// Settings are `serde_yaml::Value`, not `String`, so structured values such
/// as `viewport: [1920, 1080]` survive instead of being rejected as
/// "invalid type: sequence, expected a string".
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PartialScene {
    pub adapter: Option<String>,
    #[serde(flatten)]
    pub settings: BTreeMap<String, serde_yaml::Value>,
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

    /// This layer as it applies in `locale`: itself, then its section for
    /// that locale over it.
    pub fn in_locale(&self, locale: &str) -> Vec<PartialConfig> {
        let own = self
            .locale
            .as_ref()
            .and_then(|sections| sections.get(locale))
            .cloned();
        std::iter::once(self.clone()).chain(own).collect()
    }

    /// Builds a layer from a line's attributes.
    pub fn from_line(a: &LineAttrs) -> Self {
        Self {
            timing: Some(PartialTiming {
                lead_in_ms: a.lead_in,
                tail_ms: a.tail,
                ..PartialTiming::default()
            }),
            voice: Some(PartialVoice {
                backend: a.voice_backend.clone(),
                voice: a.voice.clone(),
                speed: a.voice_speed,
                instruct: a.voice_instruct.clone(),
                ..PartialVoice::default()
            }),
            ..Self::default()
        }
    }

    /// Builds a layer from an action block's attributes.
    pub fn from_block(a: &BlockAttrs) -> Self {
        Self {
            timing: Some(PartialTiming {
                max_stretch: a.max_stretch,
                min_stretch: a.min_stretch,
                trim_warn_above: a.trim_warn_above,
                ..PartialTiming::default()
            }),
            voice: Some(PartialVoice::default()),
            ..Self::default()
        }
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
    /// Problems only the *merged* config can see, as messages ready to be
    /// wrapped in a [`crate::Diagnostic`]. `voice.speed` can come from any
    /// layer, so only the merged value is what the estimator will be handed.
    ///
    /// A zero speed makes the estimate infinite and overflows the scheduler's
    /// padding, a panic on `check`. A negative one passes the offline paths
    /// and fails only at synthesis, so `check` would pass what `dub` refuses.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let speed = self.voice.speed;
        if !speed.is_finite() {
            out.push(format!(
                "voice.speed must be a finite number greater than zero, but is `{speed}`"
            ));
        } else if speed <= 0.0 {
            out.push(format!(
                "voice.speed must be greater than zero, but is `{speed}`"
            ));
        }
        // Zero deserializes and then divides: a zero frame rate schedules
        // no frames, and a zero-width frame is one no encoder accepts.
        if self.output.fps == 0 {
            out.push("output.fps must be greater than zero".to_string());
        }
        let (w, h) = self.output.resolution;
        if w == 0 || h == 0 {
            out.push(format!(
                "output.resolution must be greater than zero in both directions, but is `[{w}, {h}]`"
            ));
        }
        out
    }

    pub fn merged(layers: &[PartialConfig]) -> Self {
        let mut c = Config::default();
        for layer in layers {
            if let Some(l) = &layer.locales {
                set!(c.locales.source, l.source.clone());
                set!(c.locales.targets, l.targets.clone());
            }
            if let Some(v) = &layer.voice {
                set!(c.voice.backend, v.backend.clone());
                if v.voice.is_some() {
                    c.voice.voice = v.voice.clone();
                }
                set!(c.voice.speed, v.speed);
                if v.instruct.is_some() {
                    c.voice.instruct = v.instruct.clone();
                }
                if let Some(pronounce) = &v.pronounce {
                    c.voice
                        .pronounce
                        .extend(pronounce.iter().map(|(k, said)| (k.clone(), said.clone())));
                }
            }
            if let Some(t) = &layer.timing {
                set!(c.timing.lead_in_ms, t.lead_in_ms);
                set!(c.timing.tail_ms, t.tail_ms);
                set!(c.timing.max_stretch, t.max_stretch);
                set!(c.timing.min_stretch, t.min_stretch);
                set!(c.timing.trim_warn_above, t.trim_warn_above);
                set!(c.timing.min_line_speed, t.min_line_speed);
                set!(c.timing.max_line_speed, t.max_line_speed);
                set!(c.timing.min_take_speed, t.min_take_speed);
                set!(c.timing.max_take_speed, t.max_take_speed);
                if t.length_ms.is_some() {
                    c.timing.length_ms = t.length_ms;
                }
            }
            if let Some(o) = &layer.output {
                // A resolution is a pair or it is nothing: half of one is
                // not a size anything can be rendered at.
                if let Some(Resolution(w, h)) = o.resolution {
                    c.output.resolution = (w, h);
                }
                set!(c.output.fps, o.fps);
            }
            if let Some(t) = layer.output.as_ref().and_then(|o| o.transition.as_ref()) {
                set!(c.transition.kind, t.kind.clone());
                set!(c.transition.min_ms, t.min_ms);
                set!(c.transition.max_ms, t.max_ms);
                set!(c.transition.duration, t.duration.clone());
            }
            if let Some(scenes) = &layer.scene {
                set!(c.default_scene, scenes.default.clone().map(Some));
                for (name, ps) in &scenes.scenes {
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
            if let Some(t) = &layer.translate {
                set!(c.translate.provider, t.provider.clone());
                if t.model.is_some() {
                    c.translate.model = t.model.clone();
                }
            }
            if let Some(bs) = &layer.backends {
                for (id, settings) in bs {
                    // Merge the inner mapping key by key so a script
                    // overriding one setting does not discard the project's
                    // others. A non-mapping value replaces wholesale —
                    // there is nothing sensible to merge into.
                    match (c.backends.get_mut(id), settings.as_mapping()) {
                        (Some(existing), Some(new)) => {
                            if let Some(target) = existing.as_mapping_mut() {
                                for (k, v) in new {
                                    target.insert(k.clone(), v.clone());
                                }
                                continue;
                            }
                            c.backends.insert(id.clone(), settings.clone());
                        }
                        _ => {
                            c.backends.insert(id.clone(), settings.clone());
                        }
                    }
                }
            }
        }
        c
    }
}
