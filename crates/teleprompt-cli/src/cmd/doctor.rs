use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_compile::manifest::MANIFEST_VERSION;
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::VoiceRegistry;
use teleprompt_voice_kokoro::KokoroVoice;

use crate::project::Project;
use crate::voice::Backends;

/// The cache `doctor` reports on, and the path it prints for it.
///
/// Rooted at the project when there is one, exactly as `check` and `dub`
/// root theirs. A CWD-relative `.teleprompt/cache` made `doctor` report
/// `0 entries` from any subdirectory of a project with a full cache —
/// worse than saying nothing, because it looks like an answer.
///
/// `doctor` still runs outside a project (it is the command you reach for
/// when nothing else works), so the fallback is the same relative default
/// `new::scaffold` creates. The printed path says which one was used.
fn cache_root(project: Option<&Project>) -> PathBuf {
    match project {
        Some(project) => project.root.join(".teleprompt").join("cache"),
        None => PathBuf::from(".teleprompt/cache"),
    }
}

/// The project's own default `voice.backend` — what a script that never
/// overrides it would resolve to. `doctor` has no script to run the full
/// front-matter merge for, so this is deliberately shallower than
/// `teleprompt_core::program::resolve`: the project-level setting only,
/// falling back to `Config::default()`'s `"null"` both when a project
/// leaves `voice.backend` unset and when there is no project at all.
///
/// That fallback is what makes "no project" and "a project that never
/// touched voice.backend" behave identically: both end up probing whatever
/// `"null"` resolves to, which is nothing — there is no configured server
/// to ask about in either case.
fn configured_backend_id(project: Option<&Project>) -> String {
    project
        .and_then(|p| p.config.voice.as_ref())
        .and_then(|v| v.backend.clone())
        .unwrap_or_else(|| "null".to_string())
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    /// Whether the project's own files are in a state teleprompt can work
    /// from. Deliberately *not* a verdict on the machine: an unreachable
    /// server leaves this `true`, because spec §9 makes that a warning and
    /// `check`/`plan` do not need the server at all. A `backends:` block
    /// that cannot be turned into the backend the project selected is a
    /// different kind of fact — it is wrong in the repository, it is wrong
    /// on every machine, and `dub` will not run until it is fixed.
    pub ok: bool,
    pub adapters: Vec<String>,
    pub voice_backends: Vec<String>,
    pub manifest_version: u32,
    /// The cache directory these counts describe, so a reader can tell a
    /// project's cache from the empty relative default.
    pub cache_root: String,
    pub cache_entries: usize,
    pub cache_bytes: u64,
    /// One line about the project's *configured* `voice.backend`'s server
    /// (spec §9), or `None` when that backend has nothing to probe — the
    /// common case, since a freshly scaffolded project's backend is `null`.
    /// Every registered backend still appears in `voice_backends` above;
    /// only the probe is limited to the one actually selected.
    pub voice_probe: Option<String>,
    /// Everything wrong with the project's settings, in the words `check`
    /// would use. Empty on a healthy project.
    ///
    /// This used to be swallowed: `doctor` caught the construction failure,
    /// substituted default settings, and reported a healthy project — with
    /// a comment claiming `check` reported it "with a span", which was wrong
    /// on both halves. The command you run when nothing works is the last
    /// place a known problem should be hidden.
    pub problems: Vec<String>,
    pub notes: Vec<String>,
}

/// The settings-discovering entry point every caller outside a test uses:
/// discovers the project once — there is exactly one `Project::discover`
/// call in `doctor`'s whole path, here — and delegates everything else to
/// [`doctor_report_with`]. Mirrors `crate::voice::registry`/`registry_for`
/// and `crate::cmd::check::compile_script`/`compile_script_with` — the seam
/// lives on the `_with` function, and this is the thin wrapper around it.
pub async fn doctor_report(registry: &SceneRegistry) -> DoctorReport {
    let project = Project::discover(Path::new(".")).ok();
    doctor_report_with(registry, project.as_ref()).await
}

/// [`doctor_report`] against a caller-supplied project (or `None` for
/// "outside a project"), so a test can hand it an in-memory `Project`
/// pointing `backends.kokoro` at a stub server, or naming a backend other
/// than `kokoro`, without touching a real `teleprompt.toml`.
pub async fn doctor_report_with(
    registry: &SceneRegistry,
    project: Option<&Project>,
) -> DoctorReport {
    let root = cache_root(project);
    let cache = teleprompt_cache::VoiceCache::new(&root);
    let stats = cache.stats().unwrap_or(teleprompt_cache::CacheStats {
        entries: 0,
        bytes: 0,
    });

    let backends = match project {
        Some(p) => crate::cmd::check::backends_of(p),
        None => Backends::defaults(),
    };

    // Everything the settings said that could not be turned into a backend,
    // not just the part affecting the backend this project selected: unlike
    // `check`, which answers "can this script be compiled", `doctor` answers
    // "what is wrong here", and a `[backends.kokoro]` block a `null` project
    // never reads is still a block the author wrote and expected to matter.
    //
    // `ok` is the narrower question, and it is deliberately narrower: it is
    // false only for what stops *this* project working — an unmatched
    // `backends:` key, which `check` rejects outright, or a selected backend
    // whose settings will not build. A misconfigured backend nobody selects
    // is listed and does not turn the report red, the same way an
    // unreachable server is.
    let backend_id = configured_backend_id(project);
    let blocking = !backends.diagnostics().is_empty() || backends.resolve(&backend_id).is_err();
    let problems: Vec<String> = backends
        .diagnostics()
        .into_iter()
        .chain(backends.unusable_diagnostics())
        .map(|d| d.message)
        .collect();

    let voice_registry = backends.registry();

    // Spec §9: probe the *configured* backend, not every backend this
    // build happens to ship. A freshly scaffolded project's `voice.backend`
    // is `null`, which has no server — probing kokoro anyway made every
    // `doctor` run on the common case make a network call to a server
    // nothing configured, and print a line saying it was unreachable. That
    // is noise reported as a finding, and it trains people to ignore the
    // probe line.
    let voice_probe = probe_configured_backend(voice_registry, &backend_id).await;

    DoctorReport {
        ok: !blocking,
        adapters: registry.available().iter().map(|s| s.to_string()).collect(),
        voice_backends: backends.ids(),
        manifest_version: MANIFEST_VERSION,
        cache_root: root.display().to_string(),
        cache_entries: stats.entries,
        cache_bytes: stats.bytes,
        voice_probe,
        problems,
        notes: vec![
            "M0 builds no video, so ffmpeg is not required yet.".to_string(),
            "M0 ships no external runtime, so Node and Playwright are not required yet."
                .to_string(),
            // No sample rate is claimed here. `VoiceCapabilities` does not
            // carry one, so the only honest source is the audio a backend
            // actually produces — which `dub` reads per run and publishes in
            // the manifest's `audio` block. A fixed "48 kHz" here was the
            // null backend's rate quietly asserted on every backend's behalf.
            "dub writes 16-bit PCM WAV; each manifest's `audio` block records the rate and \
             channel count the backend produced."
                .to_string(),
            "the null backend renders silence of the estimated duration.".to_string(),
        ],
    }
}

/// One line about the *configured* backend's server, or `None` when it has
/// nothing to probe — either `backend_id` names something this registry
/// has nothing registered under, or what is registered there does not
/// downcast to `KokoroVoice` (today, always the `null` backend). Silence is
/// the honest answer for a backend with no server, not a "not applicable"
/// line.
///
/// Downcasting once, not twice: `VoiceRegistry::get` already hands back the
/// same `Arc` both the "is this kokoro" check and the probe itself need, so
/// there is exactly one `downcast_ref` here rather than one to test and a
/// second, unwrapped, to use.
async fn probe_configured_backend(
    voice_registry: &VoiceRegistry,
    backend_id: &str,
) -> Option<String> {
    let backend = voice_registry.get(backend_id)?;
    let kokoro = backend.as_any().downcast_ref::<KokoroVoice>()?;
    let url = kokoro.base_url().to_string();
    Some(match kokoro.voices().await {
        Ok(voices) => format!("{url} — reachable, {} voices", voices.len()),
        Err(e) => format!("{url} — unreachable ({e})"),
    })
}

impl DoctorReport {
    pub fn render(&self) -> String {
        let mut out = String::from("teleprompt doctor\n");
        out.push_str(&format!(
            "  scene adapters   {}\n",
            self.adapters.join(", ")
        ));
        out.push_str(&format!(
            "  voice backends   {}\n",
            self.voice_backends.join(", ")
        ));
        if let Some(p) = &self.voice_probe {
            out.push_str(&format!("  voice kokoro     {p}\n"));
        }
        out.push_str(&format!(
            "  manifest         narration v{}\n",
            self.manifest_version
        ));
        out.push_str(&format!(
            "  cache            {}/voice — {} entries, {} bytes\n",
            self.cache_root, self.cache_entries, self.cache_bytes
        ));
        for p in &self.problems {
            out.push_str(&format!("  problem          {p}\n"));
        }
        for n in &self.notes {
            out.push_str(&format!("  note             {n}\n"));
        }
        out
    }
}
