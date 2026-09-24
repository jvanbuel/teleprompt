use std::collections::BTreeMap;
use std::sync::Arc;

use teleprompt_core::Diagnostic;
use teleprompt_voice::{VoiceBackend, VoiceRegistry};
use teleprompt_voice_kokoro::{KokoroConfig, KokoroVoice};
use teleprompt_voice_null::NullVoice;

/// The backends this build ships, together with everything the project's
/// `backends:` settings said that could not be turned into one.
///
/// Two things live here that a bare [`VoiceRegistry`] cannot hold, and both
/// of them are failures the config layer used to have nowhere to put:
///
/// * A shipped backend whose settings do not validate. Constructing every
///   backend eagerly and propagating the first failure made
///   `[backends.kokoro] concurrency = 0` fail `check` and `plan` on a
///   project whose `voice.backend` is `null` and which will never open a
///   socket. The error is kept instead, and surfaces when that backend is
///   the one the project resolves to — which is the only run it can affect.
///   It is *kept*, not dropped: a caller that selects the backend gets the
///   original message, so this is a deferral, not a swallow.
///
/// * A `backends:` key naming nothing this build ships. `registry_for` used
///   to look up the single literal key `"kokoro"` and discard the rest
///   without a word, so `[backends.kokoro-local]` left `dub` talking to the
///   default `localhost:8880` — a server the author never configured, and
///   whose audio would be cached under `kokoro@localhost:8880` permanently.
pub struct Backends {
    registry: VoiceRegistry,
    /// Shipped backends whose settings did not validate, by id.
    unusable: BTreeMap<String, String>,
    /// `backends:` keys matching no shipped id, in the map's own order.
    unknown: Vec<String>,
    /// The file the settings came from, so a diagnostic about them can point
    /// at it rather than at a script that has nothing to do with it.
    config_file: String,
    /// The concrete Kokoro handle, kept alongside its `Arc<dyn VoiceBackend>`
    /// clone in `registry` rather than instead of it.
    ///
    /// `dub`'s voice-list validation, `dub`'s fan-out limit, and `doctor`'s
    /// probe all need an inherent method `VoiceBackend` does not carry —
    /// `voices()`, `concurrency()`, `base_url()`. Kokoro used to answer that
    /// with `as_any` and a `downcast_ref` at each of those three call sites.
    /// This is the same information reached the way `voice.rs` was already
    /// positioned to reach it: `backends_for` constructs the concrete type
    /// right here, so keeping the `Arc<KokoroVoice>` costs one field, where
    /// recovering it from the trait object afterwards cost a downcast per
    /// caller. `None` when settings named `kokoro` but it did not construct
    /// — the same case `unusable` records, so a caller that reads both never
    /// sees them disagree.
    kokoro: Option<Arc<KokoroVoice>>,
}

/// Every backend this build ships, each constructed from its own slice of
/// `backends:`. Adding one is a line here plus a crate — nothing else in the
/// workspace changes, and in particular `teleprompt-core` never learns the
/// new backend's name.
///
/// Infallible by construction, which is not the same as forgiving: each
/// failure lands in [`Backends`] and is reported by whoever is in a position
/// to say who it affects.
pub fn backends_for(settings: &BTreeMap<String, serde_yaml::Value>, config_file: &str) -> Backends {
    let mut registry = VoiceRegistry::default();
    let mut unusable = BTreeMap::new();
    let mut kokoro_handle = None;

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

    // Computed against the ids this build ships, not against the ones that
    // happened to construct: a backend whose settings are bad is still a
    // backend this build has heard of, and reporting it as unknown as well
    // would name the same mistake twice with two different explanations.
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
    }
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
    /// no project config in hand — `doctor` outside a project, and tests.
    pub fn defaults() -> Self {
        backends_for(&BTreeMap::new(), "teleprompt.toml")
    }

    /// A `Backends` wrapping a caller-supplied registry.
    ///
    /// The seam exists so a test can register a backend this build does not
    /// ship and watch the whole path — key, synthesis, cache, manifest —
    /// follow it. That is the claim spec §4.1 makes, and it is not a claim a
    /// workspace with exactly one real backend can otherwise check. Nothing
    /// is unknown or unusable here: the caller built the registry by hand,
    /// so there were no settings to misread.
    pub fn from_registry(registry: VoiceRegistry) -> Self {
        Self {
            registry,
            unusable: BTreeMap::new(),
            unknown: Vec::new(),
            config_file: "teleprompt.toml".to_string(),
            kokoro: None,
        }
    }

    /// Every id this build ships, including one whose settings did not
    /// validate — `doctor` is asked what the build can do, and a backend
    /// that exists but is misconfigured is a different answer from one that
    /// does not exist.
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

    /// The registry itself, for the two callers that need to look a backend
    /// up without resolving a project's choice.
    pub fn registry(&self) -> &VoiceRegistry {
        &self.registry
    }

    /// The concrete Kokoro handle, when `id` names it and it constructed
    /// successfully — the escape hatch `dub`'s voice-list check, `dub`'s
    /// fan-out limit, and `doctor`'s probe use to reach `voices()`,
    /// `concurrency()` and `base_url()`, none of which `VoiceBackend`
    /// carries.
    ///
    /// Gated on `id` rather than handed back unconditionally: a project can
    /// select `null` while still having a valid `[backends.kokoro]` block
    /// (or an invalid one), and none of the three callers above should act
    /// on Kokoro's server unless Kokoro is the backend actually in use. This
    /// mirrors [`resolve`](Self::resolve)'s own gating, and returns `None`
    /// on the same two occasions `resolve("kokoro")` would fail: a
    /// different id, or settings that never produced a backend.
    pub fn kokoro(&self, id: &str) -> Option<&Arc<KokoroVoice>> {
        if id == "kokoro" {
            self.kokoro.as_ref()
        } else {
            None
        }
    }

    /// The backend `id` names, or a diagnostic saying why not.
    ///
    /// The two failures are anchored differently on purpose. Bad settings
    /// are a fact about `teleprompt.toml`, so the diagnostic points there.
    /// An unknown id could equally have come from a script's own front
    /// matter, which `Backends` cannot see, so that one carries no file of
    /// its own and is rendered against whatever the caller is checking.
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
    /// An error rather than a warning, unlike the front-matter case. There
    /// the script still compiles to something correct — the project's
    /// settings for that backend, just not the script's requested variant of
    /// them. Here the author's settings reach nothing whatsoever, and the
    /// backend they were meant for keeps its defaults, so a misspelt
    /// `[backends.kokoro-local]` sends `dub` to `localhost:8880` instead of
    /// the server the author wrote down. If anything is listening there the
    /// audio is synthesized against it and cached permanently under that
    /// server's version string, reported as `measured` by every later
    /// `plan`. That is the one failure this project has singled out as its
    /// worst, and it is not a thing to warn about and carry on from.
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
    /// Separate from [`diagnostics`](Self::diagnostics) because the two
    /// answer different questions. `check` asks "can this script be
    /// compiled", and a backend it never touches has no bearing on that —
    /// that is the whole point of deferring the failure. `doctor` asks "what
    /// is wrong here", and a `backends:` block the author wrote that cannot
    /// produce a backend is wrong whichever script is being compiled today.
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
