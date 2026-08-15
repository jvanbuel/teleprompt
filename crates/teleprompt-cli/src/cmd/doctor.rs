use serde::Serialize;
use teleprompt_compile::manifest::MANIFEST_VERSION;
use teleprompt_scene::SceneRegistry;

/// Where `doctor` looks for the synthesis cache when reporting on it. Not
/// `crate::cmd::check::cache_root`: `doctor` has no `Project` to read a
/// root from (it runs without a script), so it reports on the same
/// project-relative default every project gets from `new::scaffold`.
const CACHE_ROOT: &str = ".teleprompt/cache";

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub adapters: Vec<String>,
    pub voice_backends: Vec<String>,
    pub manifest_version: u32,
    pub cache_entries: usize,
    pub cache_bytes: u64,
    pub notes: Vec<String>,
}

pub fn doctor_report(registry: &SceneRegistry) -> DoctorReport {
    let cache = teleprompt_cache::VoiceCache::new(CACHE_ROOT);
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
            "  cache            {CACHE_ROOT}/voice — {} entries, {} bytes\n",
            self.cache_entries, self.cache_bytes
        ));
        for n in &self.notes {
            out.push_str(&format!("  note             {n}\n"));
        }
        out
    }
}
