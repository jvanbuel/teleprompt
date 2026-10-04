use std::collections::BTreeMap;
use std::sync::Arc;

use teleprompt_core::Diagnostic;
use teleprompt_voice::NullVoice;
use teleprompt_voice::{VoiceBackend, VoiceRegistry};
use teleprompt_voice_gemini::{GeminiConfig, GeminiVoice};
use teleprompt_voice_kokoro::{KokoroConfig, KokoroVoice};
use teleprompt_voice_voicebox::{VoiceboxConfig, VoiceboxVoice};

/// The backends this build ships, together with everything the project's
/// `backends:` settings said that could not be turned into one.
///
/// * A shipped backend whose settings do not validate. Its error is kept and
///   surfaces only when that backend is selected, so a bad
///   `[backends.kokoro]` does not fail `check` on a `null` project.
/// * A `backends:` key naming nothing this build ships, which is an error
///   (see [`Backends::diagnostics`]).
pub struct Backends {
    registry: VoiceRegistry,
    /// Shipped backends whose settings did not validate, by id.
    unusable: BTreeMap<String, String>,
    /// `backends:` keys matching no shipped id, in the map's own order.
    unknown: Vec<String>,
    /// The file the settings came from, which diagnostics about them name.
    config_file: String,
    /// The concrete Kokoro handle, alongside its trait-object clone in
    /// `registry`; see [`Backends::kokoro`]. `None` when it did not
    /// construct.
    kokoro: Option<Arc<KokoroVoice>>,
    /// The concrete Voicebox handle; see [`Backends::voicebox`].
    voicebox: Option<Arc<VoiceboxVoice>>,
    gemini: Option<Arc<GeminiVoice>>,
}

/// Every backend this build ships, each constructed from its own slice of
/// `backends:` (docs/design.md#crates). Infallible: each failure is kept in
/// [`Backends`] for whoever knows whether it matters.
pub fn backends_for(settings: &BTreeMap<String, serde_yaml::Value>, config_file: &str) -> Backends {
    let mut registry = VoiceRegistry::default();
    let mut unusable = BTreeMap::new();
    let mut kokoro_handle = None;
    let mut voicebox_handle = None;
    let mut gemini_handle = None;

    registry.register(Arc::new(NullVoice::default()));

    match kokoro(settings.get("kokoro")) {
        Ok(v) => {
            let v = Arc::new(v);
            registry.register(v.clone());
            kokoro_handle = Some(v);
        }
        Err(e) => {
            unusable.insert("kokoro".to_string(), e);
        }
    }
    match voicebox(settings.get("voicebox")) {
        Ok(v) => {
            let v = Arc::new(v);
            registry.register(v.clone());
            voicebox_handle = Some(v);
        }
        Err(e) => {
            unusable.insert("voicebox".to_string(), e);
        }
    }
    match gemini(settings.get("gemini")) {
        Ok(v) => {
            let v = Arc::new(v);
            registry.register(v.clone());
            gemini_handle = Some(v);
        }
        Err(e) => {
            unusable.insert("gemini".to_string(), e);
        }
    }

    // Includes the unusable ones, so a bad block is not also called unknown.
    let shipped: Vec<String> = registry
        .available()
        .iter()
        .map(|s| (*s).to_string())
        .chain(unusable.keys().cloned())
        .collect();
    let unknown = settings
        .keys()
        .filter(|k| !shipped.contains(k))
        .cloned()
        .collect();

    Backends {
        registry,
        unusable,
        unknown,
        config_file: config_file.to_string(),
        kokoro: kokoro_handle,
        voicebox: voicebox_handle,
        gemini: gemini_handle,
    }
}

fn gemini(settings: Option<&serde_yaml::Value>) -> Result<GeminiVoice, String> {
    let cfg = match settings {
        Some(v) => GeminiConfig::from_value(v)?,
        None => GeminiConfig::default(),
    };
    GeminiVoice::new(cfg)
}

fn voicebox(settings: Option<&serde_yaml::Value>) -> Result<VoiceboxVoice, String> {
    let cfg = match settings {
        Some(v) => VoiceboxConfig::from_value(v)?,
        None => VoiceboxConfig::default(),
    };
    VoiceboxVoice::new(cfg)
}

fn kokoro(settings: Option<&serde_yaml::Value>) -> Result<KokoroVoice, String> {
    let cfg = match settings {
        Some(v) => KokoroConfig::from_value(v)?,
        None => KokoroConfig::default(),
    };
    KokoroVoice::new(cfg)
}

impl Backends {
    /// Settings-free, so every backend gets its defaults. For callers with
    /// no project config in hand, such as tests.
    pub fn defaults() -> Self {
        backends_for(&BTreeMap::new(), "teleprompt.toml")
    }

    /// A `Backends` wrapping a caller-supplied registry, so a test can follow
    /// a backend this build does not ship through key, synthesis, cache and
    /// manifest.
    pub fn from_registry(registry: VoiceRegistry) -> Self {
        Self {
            registry,
            unusable: BTreeMap::new(),
            unknown: Vec::new(),
            config_file: "teleprompt.toml".to_string(),
            kokoro: None,
            voicebox: None,
            gemini: None,
        }
    }

    /// Every id this build ships, including those whose settings did not
    /// validate.
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .registry
            .available()
            .iter()
            .map(|s| (*s).to_string())
            .chain(self.unusable.keys().cloned())
            .collect();
        ids.sort();
        ids
    }

    /// The registry itself, for looking a backend up without resolving a
    /// project's choice.
    pub fn registry(&self) -> &VoiceRegistry {
        &self.registry
    }

    /// The concrete Kokoro handle, for `voices()`, `concurrency()` and
    /// `base_url()`, which `VoiceBackend` does not carry.
    ///
    /// Only when `id` is `kokoro` and it constructed, so nobody acts on
    /// Kokoro's server when a project merely has a `[backends.kokoro]` block.
    pub fn kokoro(&self, id: &str) -> Option<&Arc<KokoroVoice>> {
        if id == "kokoro" {
            self.kokoro.as_ref()
        } else {
            None
        }
    }

    /// The concrete Gemini handle, for `setup`'s key check; only when `id`
    /// is `gemini` and it constructed.
    pub fn gemini(&self, id: &str) -> Option<&Arc<GeminiVoice>> {
        self.gemini.as_ref().filter(|_| id == "gemini")
    }

    /// The concrete Voicebox handle, for its profiles, concurrency and
    /// address; only when `id` is `voicebox` and it constructed.
    pub fn voicebox(&self, id: &str) -> Option<&Arc<VoiceboxVoice>> {
        if id == "voicebox" {
            self.voicebox.as_ref()
        } else {
            None
        }
    }

    /// Lines `dub` sends the backend `id` at once.
    pub fn concurrency(&self, id: &str) -> usize {
        self.kokoro(id)
            .map(|k| k.concurrency())
            .or_else(|| self.voicebox(id).map(|v| v.concurrency()))
            .or_else(|| self.gemini(id).map(|g| g.concurrency()))
            .unwrap_or(1)
    }

    /// The backend `id` names, or a diagnostic saying why not.
    ///
    /// Bad settings point at `teleprompt.toml`. An unknown id may come from
    /// a script's front matter, so it carries no file and is rendered
    /// against whatever the caller is checking.
    pub fn resolve(&self, id: &str) -> Result<Arc<dyn VoiceBackend>, Diagnostic> {
        if let Some(b) = self.registry.get(id) {
            return Ok(b);
        }
        if let Some(why) = self.unusable.get(id) {
            return Err(Diagnostic::error(format!(
                "this project's voice backend is `{id}`, and its settings are not \
                 usable: {why}"
            ))
            .in_file(self.config_file.clone()));
        }
        Err(Diagnostic::error(format!(
            "unknown voice backend `{id}` (available: {})",
            self.ids().join(", ")
        )))
    }

    /// One error per `backends:` key naming nothing this build ships.
    ///
    /// An error, not a warning: the settings reach nothing, so a misspelt
    /// `[backends.kokoro-local]` sends `dub` to the default server, whose
    /// audio is then cached and reported as `measured` by every later
    /// `plan`.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.unknown
            .iter()
            .map(|id| {
                Diagnostic::error(format!(
                    "`backends.{id}` sets options for a voice backend this build does \
                     not ship, so nothing reads them (this build ships: {})",
                    self.ids().join(", ")
                ))
                .in_file(self.config_file.clone())
                .with_help(
                    "remove the block, or rename it to the backend id you meant — a \
                     backend whose settings never arrive falls back to its defaults \
                     rather than failing",
                )
            })
            .collect()
    }

    /// One error per shipped backend whose settings did not validate,
    /// whether or not this project selects it.
    ///
    /// Separate from [`diagnostics`](Self::diagnostics): `check` asks
    /// whether this script compiles, which an unselected backend cannot
    /// affect; `setup` asks what is wrong, which it is.
    pub(crate) fn unusable_diagnostics(&self) -> Vec<Diagnostic> {
        self.unusable
            .iter()
            .map(|(id, why)| {
                Diagnostic::error(format!("`backends.{id}` is not usable: {why}"))
                    .in_file(self.config_file.clone())
            })
            .collect()
    }
}

/// Shorter than a backend's own `timeout_ms`, which is sized for synthesis:
/// listing voices runs no model, so a server this slow to answer is broken.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(5_000);

impl Backends {
    /// What backend `id`'s server says, in one line: its address, whether it
    /// answers, and its model and voices; `None` for a backend with no
    /// server, such as `null`, or one whose settings did not build.
    pub async fn probe(&self, id: &str) -> Option<String> {
        let unreachable = |url: &str| {
            format!(
                "{url} — unreachable (no response within {}ms)",
                PROBE_TIMEOUT.as_millis()
            )
        };
        if let Some(voicebox) = self.voicebox(id) {
            let url = voicebox.base_url().to_string();
            return Some(
                match tokio::time::timeout(PROBE_TIMEOUT, voicebox.profiles()).await {
                    Ok(Ok(profiles)) => format!(
                        "{url} — reachable, {}, voices: {}",
                        voicebox.capabilities().version,
                        if profiles.is_empty() {
                            "none yet (`teleprompt voice clone`)".to_string()
                        } else {
                            let names: Vec<&str> =
                                profiles.iter().map(|p| p.name.as_str()).collect();
                            names.join(", ")
                        }
                    ),
                    // In the backend's own words, which name the address: a
                    // refused connection, a 500 and a bad voice list are
                    // different fixes.
                    Ok(Err(e)) => e.to_string(),
                    Err(_) => unreachable(&url),
                },
            );
        }
        if let Some(gemini) = self.gemini(id) {
            return Some(
                match tokio::time::timeout(PROBE_TIMEOUT, gemini.check()).await {
                    Ok(Ok(line)) => line,
                    Ok(Err(e)) => e.to_string(),
                    Err(_) => unreachable("gemini"),
                },
            );
        }
        let kokoro = self.kokoro(id)?;
        let url = kokoro.base_url().to_string();
        Some(
            match tokio::time::timeout(PROBE_TIMEOUT, kokoro.voices()).await {
                // The model beside the address: docs/design.md#voice-cache.
                Ok(Ok(voices)) => format!(
                    "{url} — reachable, model {}, {} voices",
                    kokoro.capabilities().version,
                    voices.len()
                ),
                Ok(Err(e)) => e.to_string(),
                Err(_) => unreachable(&url),
            },
        )
    }
}
