use std::collections::BTreeMap;
use std::sync::Arc;

use teleprompt_core::Diagnostic;
use teleprompt_plugin::tool::Tool;
use teleprompt_voice::NullVoice;
use teleprompt_voice::VoiceRegistry;
use teleprompt_voice::{ClonedVoice, Provider, VoiceBackend, VoiceSample};

mod needs;

/// Every voice this build ships, `null` aside, in the order errors and
/// `setup` list them, with what each needs that teleprompt does not ship.
fn providers() -> Vec<(Provider, &'static Tool)> {
    vec![
        (teleprompt_voice_openai::kokoro(), &needs::KOKORO),
        (teleprompt_voice_openai::openai(), &needs::OPENAI),
        (teleprompt_voice_voicebox::provider(), &needs::VOICEBOX),
        (teleprompt_voice_gemini::provider(), &needs::GEMINI),
        (teleprompt_voice_elevenlabs::provider(), &needs::ELEVENLABS),
    ]
}

/// What the voices teleprompt ships need that it does not: their servers,
/// or keys.
pub fn needs() -> Vec<&'static Tool> {
    providers().into_iter().map(|(_, needs)| needs).collect()
}

/// The voices this build ships, `null` aside, each with what it needs.
pub fn shipped() -> Vec<(&'static str, &'static Tool)> {
    providers()
        .into_iter()
        .map(|(p, needs)| (p.id, needs))
        .collect()
}

/// Whether `name` is a voice this build ships.
pub fn is_built_in(name: &str) -> bool {
    name == "null" || providers().iter().any(|(p, _)| p.id == name)
}

/// The backends this build ships, together with everything the project's
/// `backends:` settings said that could not be turned into one.
///
/// * A shipped backend whose settings do not validate. Its error is kept and
///   surfaces only when that backend is selected, so a bad
///   `[backends.kokoro]` does not fail `check` on a `null` project.
/// * A `backends:` key naming no voice this build ships, which is a server
///   of the author's, and whose settings do not make one: an error whether
///   or not it is chosen (see [`Backends::diagnostics`]).
#[derive(Clone)]
pub struct Backends {
    registry: VoiceRegistry,
    /// Shipped backends whose settings did not validate, by id.
    unusable: BTreeMap<String, String>,
    /// `backends:` keys naming no shipped voice whose settings do not make
    /// a server, in the map's own order.
    unknown: Vec<String>,
    /// The file the settings came from, which diagnostics about them name.
    config_file: String,
}

/// Every backend this build ships, each built from its own slice of
/// `backends:` (docs/design.md#crates). Infallible: each failure is kept in
/// [`Backends`] for whoever knows whether it matters.
pub fn backends_for(settings: &BTreeMap<String, serde_yaml::Value>, config_file: &str) -> Backends {
    let mut registry = VoiceRegistry::default();
    let mut unusable = BTreeMap::new();
    registry.register(Arc::new(NullVoice::default()));
    for (provider, _) in providers() {
        match (provider.build)(settings.get(provider.id)) {
            Ok(backend) => registry.register(backend),
            Err(e) => {
                unusable.insert(provider.id.to_string(), e);
            }
        }
    }
    // Any other is a server of the author's that speaks OpenAI's API,
    // under the name they gave it.
    let mut unknown = Vec::new();
    for (name, given) in settings {
        if is_built_in(name) {
            continue;
        }
        match teleprompt_voice_openai::endpoint(name, given) {
            Ok(backend) => registry.register(backend),
            Err(e) => {
                unusable.insert(name.clone(), e);
                unknown.push(name.clone());
            }
        }
    }

    Backends {
        registry,
        unusable,
        unknown,
        config_file: config_file.to_string(),
    }
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

    /// Lines `dub` sends the backend `id` at once.
    pub fn concurrency(&self, id: &str) -> usize {
        self.registry.get(id).map_or(1, |b| b.concurrency())
    }

    /// A voice named `name`, cloned from `samples` by the first backend
    /// that clones voices, and which one; why not, when none here does.
    pub async fn clone_voice(
        &self,
        name: &str,
        language: &str,
        samples: &[VoiceSample],
    ) -> Result<(String, ClonedVoice), String> {
        for id in self.registry.available() {
            let Some(backend) = self.registry.get(id) else {
                continue;
            };
            match backend.clone_voice(name, language, samples).await {
                Err(teleprompt_voice::VoiceError::Unsupported { .. }) => continue,
                cloned => {
                    return cloned
                        .map(|c| (id.to_string(), c))
                        .map_err(|e| e.to_string())
                }
            }
        }
        let mut why = "no voice backend here clones a voice".to_string();
        for (id, problem) in &self.unusable {
            why.push_str(&format!(
                "; `[backends.{id}]` in {} does not validate: {problem}",
                self.config_file
            ));
        }
        Err(why)
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

    /// One error per `backends:` key naming no voice this build ships whose
    /// settings do not make a server.
    ///
    /// An error whether or not it is chosen: the only reason to write one
    /// is a server of your own, and a misspelt `[backends.kokoro]` would
    /// otherwise send `dub` to the default server, whose audio is then
    /// cached and reported as `measured` by every later `plan`.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.unknown
            .iter()
            .map(|id| {
                let why = self.unusable.get(id).cloned().unwrap_or_default();
                Diagnostic::error(format!(
                    "`backends.{id}` names no voice this build ships ({}), so it is a \
                     speech server of your own, and {why}",
                    self.ids_shipped().join(", ")
                ))
                .in_file(self.config_file.clone())
                .with_help(
                    "for a server that speaks OpenAI's speech API, give its `base_url`; \
                     otherwise rename the block to the voice you meant",
                )
            })
            .collect()
    }

    /// The voices this build ships, whatever the project configures.
    fn ids_shipped(&self) -> Vec<&'static str> {
        std::iter::once("null")
            .chain(providers().into_iter().map(|(p, _)| p.id))
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

/// Each backend a line is spoken by, by id.
pub type Voices = BTreeMap<String, Arc<dyn VoiceBackend>>;

impl Backends {
    /// The backends `narration` is spoken by: `main`, and each a speaker's
    /// line picks. The compile has already found them all.
    pub fn voices(
        &self,
        main: &Arc<dyn VoiceBackend>,
        narration: &[teleprompt_compile::NarrationDetail],
    ) -> Result<Voices, String> {
        let mut voices = Voices::new();
        voices.insert(main.id().to_string(), main.clone());
        for detail in narration {
            if !voices.contains_key(&detail.backend) {
                let backend = self
                    .resolve(&detail.backend)
                    .map_err(|d| format!("line `{}`: {}", detail.line_id, d.message))?;
                voices.insert(detail.backend.clone(), backend);
            }
        }
        Ok(voices)
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
        let backend = self.registry.get(id)?;
        let address = backend.address()?;
        Some(
            match tokio::time::timeout(PROBE_TIMEOUT, backend.probe()).await {
                Ok(Ok(line)) => line,
                // In the backend's own words, which name the address: a
                // refused connection, a 500 and a bad voice list are
                // different fixes.
                Ok(Err(e)) => e.to_string(),
                Err(_) => format!(
                    "{address} — unreachable (no response within {}ms)",
                    PROBE_TIMEOUT.as_millis()
                ),
            },
        )
    }
}
