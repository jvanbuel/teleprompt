//! The project file, `teleprompt.toml`, and the layers over it: front
//! matter, a chapter's settings, a line's or block's attributes, each an
//! all-optional [`PartialConfig`] merged into one [`Config`]
//! (docs/design.md#configuration). The vocabulary it is written in is
//! core's, re-exported here.

use std::collections::BTreeMap;

use serde::Deserialize;

use teleprompt_core::attrs::{BlockAttrs, LineAttrs};
pub use teleprompt_core::config::*;
use teleprompt_core::DurationMs;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub locales: Locales,
    pub voice: VoiceConfig,
    pub timing: TimingConfig,
    pub output: OutputConfig,
    pub transition: TransitionConfig,
    pub scenes: BTreeMap<String, SceneConfig>,
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
    /// The cast: each speaker's voice, as it differs from `voice`.
    pub voices: BTreeMap<String, PartialVoice>,
    /// Who says a line that names no speaker; the narrator when none.
    pub speaker: Option<String>,
    /// The project's directory, which relative paths in scene settings are
    /// relative to. Empty, they are relative to where teleprompt runs.
    pub root: std::path::PathBuf,
}

/// What `translate` translates with.
#[derive(Debug, Clone, PartialEq)]
pub struct TranslateConfig {
    pub provider: String,
    /// The provider's model; its own default when unset.
    pub model: Option<String>,
    /// How long one batch may take, whichever the provider.
    pub timeout_ms: u64,
    /// `[translate.ollama]`: each provider's own settings (where to reach
    /// it, a key), by provider. They are apart from `[backends]`, which are
    /// the voices', so `openai` can name both a voice and a translator.
    pub settings: BTreeMap<String, serde_yaml::Value>,
}

/// A model on a laptop can take minutes over twenty lines.
pub const TRANSLATE_TIMEOUT_MS: u64 = 600_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Locales {
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceConfig {
    /// Who speaks in this voice, as the video names them on screen; for a
    /// speaker, their key title-cased when not given.
    pub name: Option<String>,
    pub backend: String,
    pub voice: Option<String>,
    pub speed: f64,
    /// How to deliver each line, for a backend that takes instructions.
    pub instruct: Option<String>,
    /// How to say words a voice gets wrong, applied to synthesis only.
    /// The script, the captions and the manifest keep the spelling.
    pub pronounce: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            locales: Locales {
                source: "en".into(),
            },
            voice: VoiceConfig {
                name: None,
                backend: "null".into(),
                voice: None,
                speed: 1.0,
                instruct: None,
                pronounce: BTreeMap::new(),
            },
            output: OutputConfig {
                resolution: (1920, 1080),
                fps: 30,
                names: true,
            },
            timing: TimingConfig {
                lead_in_ms: DurationMs::millis(150),
                tail_ms: DurationMs::millis(150),
                turn_gap_ms: DurationMs::millis(250),
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
            backends: BTreeMap::new(),
            // Local, so a script is translated on the machine by default.
            translate: TranslateConfig {
                provider: "ollama".into(),
                model: None,
                timeout_ms: TRANSLATE_TIMEOUT_MS,
                settings: BTreeMap::new(),
            },
            voices: BTreeMap::new(),
            speaker: None,
            root: std::path::PathBuf::new(),
        }
    }
}

/// All-optional mirror of `Config`, deserialized from one configuration layer
/// (`teleprompt.toml`, script/chapter front matter, or line/block attributes).
/// An unknown key is an error, so a setting nothing reads is not written.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialConfig {
    /// `teleprompt: 1` in a script's front matter: the format's version, and
    /// what tells an editor (`teleprompt lsp`) the file is a script when it
    /// has no `teleprompt` block yet. Not configuration.
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
    /// `[voices.guest]`: the cast, a voice per speaker, over `voice`.
    pub voices: Option<BTreeMap<String, PartialVoice>>,
    /// `speaker: guest`: who says the lines that name no speaker.
    pub speaker: Option<String>,
    /// The directory this layer's file is in, for a project's
    /// `teleprompt.toml`: set by whoever read it, never written in it.
    #[serde(skip)]
    pub root: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialTranslate {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub timeout_ms: Option<u64>,
    // The providers are teleprompt's own, so they are named here and a
    // misspelt one is an error rather than a setting nothing reads.
    pub ollama: Option<serde_yaml::Value>,
    pub openai: Option<serde_yaml::Value>,
    pub command: Option<serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialLocales {
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialVoice {
    /// Who this is, as the video names them on screen: `Ada Lovelace`.
    pub name: Option<String>,
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
    pub turn_gap_ms: Option<DurationMs>,
    pub max_stretch: Option<f64>,
    pub min_stretch: Option<f64>,
    pub trim_warn_above: Option<f64>,
    pub min_line_speed: Option<f64>,
    pub max_line_speed: Option<f64>,
    pub min_take_speed: Option<f64>,
    pub max_take_speed: Option<f64>,
    pub length_ms: Option<DurationMs>,
}

/// The `output:` front-matter block: frame size, rate and transitions.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialOutput {
    pub resolution: Option<Resolution>,
    pub fps: Option<u32>,
    pub names: Option<bool>,
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

/// The `scene:` block: each scene by name, with its plugin and settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct PartialScenes {
    pub scenes: BTreeMap<String, PartialScene>,
}

/// Settings are `serde_yaml::Value`, not `String`, so structured values such
/// as `viewport: [1920, 1080]` survive instead of being rejected as
/// "invalid type: sequence, expected a string".
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PartialScene {
    /// The scene plugin that records it.
    pub plugin: Option<String>,
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

    /// Builds a layer from settings written `path.to.key=value`, as on a
    /// chapter's heading, each value read as YAML would read it.
    pub fn from_settings(settings: &[(String, String)]) -> Result<Self, ConfigError> {
        use serde_yaml::{Mapping, Value};
        let mut root = Mapping::new();
        for (path, value) in settings {
            let value: Value =
                serde_yaml::from_str(value).unwrap_or_else(|_| Value::String(value.clone()));
            let mut keys: Vec<&str> = path.split('.').collect();
            let last = keys.pop().unwrap_or_default();
            let mut at = &mut root;
            for key in keys {
                let entry = at
                    .entry(Value::String(key.to_string()))
                    .or_insert_with(|| Value::Mapping(Mapping::new()));
                if !entry.is_mapping() {
                    *entry = Value::Mapping(Mapping::new());
                }
                let Value::Mapping(inner) = entry else {
                    unreachable!("made a mapping")
                };
                at = inner;
            }
            at.insert(Value::String(last.to_string()), value);
        }
        Ok(serde_yaml::from_value(Value::Mapping(root))?)
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

/// Each named field of `$src` that is set, over the same field of
/// `$target`: `set!(a, b, x, y)`. A field that is an `Option` in both is
/// listed after `opt`.
macro_rules! set {
    ($target:expr, $src:expr $(, $field:ident)* $(; opt $($opt:ident),*)?) => {
        $(
            if let Some(v) = &$src.$field {
                $target.$field = v.clone();
            }
        )*
        $($(
            if $src.$opt.is_some() {
                $target.$opt = $src.$opt.clone();
            }
        )*)?
    };
}

impl PartialVoice {
    /// `over`'s settings over these, a pronunciation at a time.
    pub fn merge(&mut self, over: &PartialVoice) {
        set!(self, over; opt name, backend, voice, speed, instruct);
        if let Some(words) = &over.pronounce {
            self.pronounce
                .get_or_insert_with(BTreeMap::new)
                .extend(words.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
}

impl VoiceConfig {
    /// `v` over this voice, field by field; `pronounce` word by word.
    fn apply(&mut self, v: &PartialVoice) {
        set!(self, v, backend, speed; opt name, voice, instruct);
        if let Some(pronounce) = &v.pronounce {
            self.pronounce
                .extend(pronounce.iter().map(|(k, said)| (k.clone(), said.clone())));
        }
    }
}

/// Why `locale` is not a language tag (`en`, `pt-BR`, `zh_Hant`), if it is
/// not. A locale names a translation beside the script and a directory
/// under `dub --out`, so a slash or a dot could name any file.
pub fn locale_problem(locale: &str) -> Option<String> {
    let tag = locale.len() <= 35
        && locale.starts_with(|c: char| c.is_ascii_alphabetic())
        && locale
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    (!tag).then(|| {
        format!("`{locale}` is not a locale: use a language tag such as `en`, `nl` or `pt-BR`")
    })
}

impl Config {
    /// Problems only the *merged* config can see, as messages ready to be
    /// wrapped in a [`teleprompt_core::Diagnostic`]. `voice.speed` can come from any
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
        if self.translate.timeout_ms == 0 {
            out.push("translate.timeout_ms must be greater than zero".to_string());
        }
        let (w, h) = self.output.resolution;
        if w == 0 || h == 0 {
            out.push(format!(
                "output.resolution must be greater than zero in both directions, but is `[{w}, {h}]`"
            ));
        }
        let t = &self.timing;
        for (what, min, max) in [
            ("line", t.min_line_speed, t.max_line_speed),
            ("take", t.min_take_speed, t.max_take_speed),
        ] {
            if !(min.is_finite() && max.is_finite() && min > 0.0 && min <= max) {
                out.push(format!(
                    "timing.min_{what}_speed and timing.max_{what}_speed must be finite, \
                     above zero, and the least no more than the most, but are `{min}` and `{max}`"
                ));
            }
        }
        out.extend(locale_problem(&self.locales.source));
        out
    }

    pub fn merged(layers: &[PartialConfig]) -> Self {
        let mut c = Config::default();
        for layer in layers {
            set!(c, layer, root; opt speaker);
            if let Some(l) = &layer.locales {
                set!(c.locales, l, source);
            }
            if let Some(v) = &layer.voice {
                c.voice.apply(v);
            }
            if let Some(t) = &layer.timing {
                set!(c.timing, t, lead_in_ms, tail_ms, turn_gap_ms, max_stretch, min_stretch,
                    trim_warn_above, min_line_speed, max_line_speed, min_take_speed,
                    max_take_speed; opt length_ms);
            }
            if let Some(o) = &layer.output {
                // A resolution is a pair or it is nothing: half of one is
                // not a size anything can be rendered at.
                if let Some(Resolution(w, h)) = o.resolution {
                    c.output.resolution = (w, h);
                }
                set!(c.output, o, fps, names);
                if let Some(t) = &o.transition {
                    set!(c.transition, t, kind, duration, min_ms, max_ms);
                }
            }
            if let Some(scenes) = &layer.scene {
                for (name, ps) in &scenes.scenes {
                    let entry = c.scenes.entry(name.clone()).or_insert_with(|| SceneConfig {
                        // Undeclared, a scene is the plugin of its own name.
                        plugin: name.clone(),
                        settings: BTreeMap::new(),
                        root: Default::default(),
                    });
                    set!(entry, ps, plugin);
                    for (k, v) in &ps.settings {
                        entry.settings.insert(k.clone(), v.clone());
                    }
                }
            }
            if let Some(cast) = &layer.voices {
                for (name, voice) in cast {
                    let entry = c.voices.entry(name.clone()).or_default();
                    entry.merge(voice);
                }
            }
            if let Some(t) = &layer.translate {
                set!(c.translate, t, provider, timeout_ms; opt model);
                let providers = [
                    ("ollama", &t.ollama),
                    ("openai", &t.openai),
                    ("command", &t.command),
                ];
                for (id, settings) in providers {
                    if let Some(settings) = settings {
                        merge_settings(&mut c.translate.settings, id, settings);
                    }
                }
            }
            if let Some(bs) = &layer.backends {
                for (id, settings) in bs {
                    merge_settings(&mut c.backends, id, settings);
                }
            }
        }
        for scene in c.scenes.values_mut() {
            scene.root = c.root.clone();
        }
        c
    }
}

/// Settings for `id` laid over what an earlier layer set, key by key, so a
/// script overriding one setting does not discard the project's others. A
/// non-mapping value replaces wholesale: there is nothing to merge into.
fn merge_settings(
    all: &mut BTreeMap<String, serde_yaml::Value>,
    id: &str,
    settings: &serde_yaml::Value,
) {
    if let (Some(target), Some(new)) = (
        all.get_mut(id).and_then(|v| v.as_mapping_mut()),
        settings.as_mapping(),
    ) {
        for (k, v) in new {
            target.insert(k.clone(), v.clone());
        }
        return;
    }
    all.insert(id.to_string(), settings.clone());
}
