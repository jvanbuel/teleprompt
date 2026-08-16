use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_compile::manifest::MANIFEST_VERSION;
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::VoiceRegistry;
use teleprompt_voice_kokoro::KokoroVoice;

use crate::project::Project;

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
fn cache_root() -> PathBuf {
    match Project::discover(Path::new(".")) {
        Ok(project) => project.root.join(".teleprompt").join("cache"),
        Err(_) => PathBuf::from(".teleprompt/cache"),
    }
}

/// The project's `backends:` settings, discovered the same way
/// [`cache_root`] discovers the project itself: rooted at the project when
/// there is one, empty outside of one so a bad or missing project does not
/// stop `doctor` from saying anything.
fn backend_settings() -> BTreeMap<String, serde_yaml::Value> {
    match Project::discover(Path::new(".")) {
        Ok(project) => project.config.backends.unwrap_or_default(),
        Err(_) => BTreeMap::new(),
    }
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub adapters: Vec<String>,
    pub voice_backends: Vec<String>,
    pub manifest_version: u32,
    /// The cache directory these counts describe, so a reader can tell a
    /// project's cache from the empty relative default.
    pub cache_root: String,
    pub cache_entries: usize,
    pub cache_bytes: u64,
    /// One line about the configured backend's server, or `None` when the
    /// resolved backend has nothing to probe (`null`).
    pub voice_probe: Option<String>,
    pub notes: Vec<String>,
}

/// The settings-discovering entry point every caller outside a test uses:
/// finds the project's own `backends:` (or falls back to defaults outside a
/// project) and delegates to [`doctor_report_with`]. Mirrors
/// `crate::voice::registry`/`registry_for` and
/// `crate::cmd::check::compile_script`/`compile_script_with` — the seam
/// lives on the `_with` function, and this is the thin wrapper around it.
pub async fn doctor_report(registry: &SceneRegistry) -> DoctorReport {
    doctor_report_with(registry, &backend_settings()).await
}

/// [`doctor_report`] against caller-supplied backend settings, so a test can
/// point `doctor` at a stub server instead of whatever the discovered
/// project (or its absence) would otherwise resolve.
pub async fn doctor_report_with(
    registry: &SceneRegistry,
    backends: &BTreeMap<String, serde_yaml::Value>,
) -> DoctorReport {
    let root = cache_root();
    let cache = teleprompt_cache::VoiceCache::new(&root);
    let stats = cache.stats().unwrap_or(teleprompt_cache::CacheStats {
        entries: 0,
        bytes: 0,
    });

    let voice_registry = crate::voice::registry_for(backends).unwrap_or_else(|_| {
        // A bad `backends:` value is reported by `check`, with a span.
        // `doctor` is the command you run when nothing else works, so it
        // falls back to defaults rather than refusing to say anything.
        crate::voice::registry()
    });

    let voice_probe = probe_kokoro(&voice_registry).await;

    DoctorReport {
        ok: true,
        adapters: registry.available().iter().map(|s| s.to_string()).collect(),
        voice_backends: voice_registry
            .available()
            .iter()
            .map(|s| s.to_string())
            .collect(),
        manifest_version: MANIFEST_VERSION,
        cache_root: root.display().to_string(),
        cache_entries: stats.entries,
        cache_bytes: stats.bytes,
        voice_probe,
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

/// One line about the kokoro server's reachability, or `None` when the
/// registry has nothing under `kokoro` to probe — the resolved backend is
/// `null` (or some other non-kokoro backend), which has no server, so
/// silence is the honest answer rather than a "not applicable" line.
///
/// Downcasting once, not twice: `VoiceRegistry::get` already hands back the
/// same `Arc` both the "is this kokoro" check and the probe itself need, so
/// there is exactly one `downcast_ref` here rather than one to test and a
/// second, unwrapped, to use.
async fn probe_kokoro(voice_registry: &VoiceRegistry) -> Option<String> {
    let backend = voice_registry.get("kokoro")?;
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
        for n in &self.notes {
            out.push_str(&format!("  note             {n}\n"));
        }
        out
    }
}
