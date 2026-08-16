use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_compile::manifest::MANIFEST_VERSION;
use teleprompt_scene::SceneRegistry;

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
    pub notes: Vec<String>,
}

pub fn doctor_report(registry: &SceneRegistry) -> DoctorReport {
    let root = cache_root();
    let cache = teleprompt_cache::VoiceCache::new(&root);
    let stats = cache.stats().unwrap_or(teleprompt_cache::CacheStats {
        entries: 0,
        bytes: 0,
    });

    DoctorReport {
        ok: true,
        adapters: registry.available().iter().map(|s| s.to_string()).collect(),
        voice_backends: crate::voice::registry()
            .available()
            .iter()
            .map(|s| s.to_string())
            .collect(),
        manifest_version: MANIFEST_VERSION,
        cache_root: root.display().to_string(),
        cache_entries: stats.entries,
        cache_bytes: stats.bytes,
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
