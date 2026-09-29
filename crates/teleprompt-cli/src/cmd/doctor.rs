use std::path::Path;

use serde::Serialize;
use teleprompt_manifest::MANIFEST_VERSION;
use teleprompt_scene::SceneRegistry;

use crate::project::{CacheDirs, Project};
use crate::voice::Backends;

/// The cache `doctor` reports on: the project's, rooted as `check` and `dub`
/// root theirs, so a subdirectory does not report an empty cache. Outside a
/// project `doctor` still runs, and falls back to the relative default.
fn caches(project: Option<&Project>) -> CacheDirs {
    match project {
        Some(project) => project.caches(),
        None => CacheDirs::under(Path::new("")),
    }
}

/// The project-level `voice.backend`, or `"null"` when it is unset or there
/// is no project. Shallower than `teleprompt_core::program::resolve` on
/// purpose: `doctor` has no script whose front matter it could merge.
fn configured_backend_id(project: Option<&Project>) -> String {
    project
        .and_then(|p| p.config.voice.as_ref())
        .and_then(|v| v.backend.clone())
        .unwrap_or_else(|| "null".to_string())
}

/// One backend's server, and what it said. The id is carried rather than
/// implied, so the label cannot name a different backend from the answer.
#[derive(Debug, Serialize)]
pub struct VoiceProbe {
    pub backend: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureBackendStatus {
    pub id: String,
    pub adapter: String,
    /// Why it cannot run here, or `null` when it can.
    pub unavailable: Option<String>,
    /// The command that says how to install what it needs, when it cannot.
    pub fix: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    /// Whether the project's own files are usable. Not a verdict on the
    /// machine: an unreachable server or a missing ffmpeg leaves it `true`
    /// (docs/design.md#backend-failure). An unknown `backends:` key or a
    /// selected backend that will not build makes it `false`.
    pub ok: bool,
    pub adapters: Vec<String>,
    /// What can *run* a scene on this machine, as against compile one. A
    /// scene with no available backend renders as a slate.
    pub capture_backends: Vec<CaptureBackendStatus>,
    pub voice_backends: Vec<String>,
    pub manifest_version: u32,
    /// The cache directory these counts describe.
    pub cache_root: String,
    pub cache_entries: usize,
    pub cache_bytes: u64,
    /// The same for the compose cache, reported separately because it is
    /// the one that grows large and the one `teleprompt cache` can shrink.
    pub compose_entries: usize,
    pub compose_bytes: u64,
    /// ffmpeg's version line, or `None` when there is no ffmpeg. Only
    /// `build` needs it.
    pub ffmpeg: Option<String>,
    /// What the *configured* backend's server said, or `None` when it has no
    /// server (as with a new project's `null`). Only that backend is probed.
    pub voice_probe: Option<VoiceProbe>,
    /// Everything wrong with the project's settings, in the words `check`
    /// would use, including backends the project does not select.
    pub problems: Vec<String>,
    pub notes: Vec<String>,
}

/// [`doctor_report_with`] for the project containing the current directory,
/// if any. The only place `doctor` discovers a project.
pub async fn doctor_report(registry: &SceneRegistry) -> DoctorReport {
    let project = Project::discover(Path::new(".")).ok();
    doctor_report_with(registry, project.as_ref()).await
}

/// [`doctor_report`] against a caller-supplied project (`None` for outside
/// one), so a test can point it at a stub server without a real project.
pub async fn doctor_report_with(
    registry: &SceneRegistry,
    project: Option<&Project>,
) -> DoctorReport {
    let caches = caches(project);
    let root = caches.root.clone();
    let cache = teleprompt_cache::VoiceCache::new(&root);
    let stats = cache.stats().unwrap_or(teleprompt_cache::CacheStats {
        entries: 0,
        bytes: 0,
    });

    let compose = crate::cmd::cache::stats(&caches.compose());

    let backends = match project {
        Some(p) => crate::cmd::check::backends_of(p),
        None => Backends::defaults(),
    };

    // `problems` lists every unusable `backends:` block (see
    // `Backends::unusable_diagnostics`). `ok` is narrower: only an unknown
    // key, which `check` rejects, or a selected backend that will not build.
    let backend_id = configured_backend_id(project);
    let blocking = !backends.diagnostics().is_empty() || backends.resolve(&backend_id).is_err();
    let problems: Vec<String> = backends
        .diagnostics()
        .into_iter()
        .chain(backends.unusable_diagnostics())
        .map(|d| d.message)
        .collect();

    // Only the configured backend: probing a server nothing configured
    // reports an unreachable server as a finding on every `null` project.
    let voice_probe = probe_configured_backend(&backends, &backend_id).await;

    DoctorReport {
        ok: !blocking,
        adapters: registry.available().iter().map(|s| s.to_string()).collect(),
        capture_backends: capture_backends(),
        voice_backends: backends.ids(),
        manifest_version: MANIFEST_VERSION,
        cache_root: root.display().to_string(),
        cache_entries: stats.entries,
        cache_bytes: stats.bytes,
        compose_entries: compose.entries,
        compose_bytes: compose.bytes,
        ffmpeg: probe_ffmpeg("ffmpeg"),
        voice_probe,
        problems,
        notes: vec![
            "build renders with ffmpeg as a subprocess; check, plan, diff and dub need none."
                .to_string(),
            "an item whose scene nothing here can record holds its slot with a slate; \
             `capture` says which."
                .to_string(),
            "no external tool ships with teleprompt: each is under its own license, and needed only by what uses it. `teleprompt setup` says which are here, their licenses, and how to install the rest."
                .to_string(),
            // No sample rate: `VoiceCapabilities` carries none, and only the
            // audio a backend produces can say.
            "dub writes 16-bit PCM WAV; each manifest's `audio` block records the rate and \
             channel count the backend produced."
                .to_string(),
            "the null backend renders silence of the estimated duration.".to_string(),
        ],
    }
}

/// The first line of `program -version`, or `None` when it cannot run. The
/// version matters: a too-old ffmpeg fails halfway through a render.
pub fn probe_ffmpeg(program: &str) -> Option<String> {
    let out = std::process::Command::new(program)
        .arg("-version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|line| line.trim().to_string())
}

/// Voicebox's address, engine and voices: which profiles a script can name.
async fn probe_voicebox(
    voicebox: &teleprompt_voice_voicebox::VoiceboxVoice,
    backend_id: &str,
) -> VoiceProbe {
    let url = voicebox.base_url().to_string();
    let detail = match tokio::time::timeout(PROBE_TIMEOUT, voicebox.profiles()).await {
        Ok(Ok(profiles)) => format!(
            "{url} — reachable, {}, voices: {}",
            teleprompt_voice::VoiceBackend::capabilities(voicebox).version,
            if profiles.is_empty() {
                "none yet (`teleprompt voice clone`)".to_string()
            } else {
                profiles
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ),
        // In the backend's own words, which name the address: a refused
        // connection, a 500 and a bad voice list are different fixes.
        Ok(Err(e)) => e.to_string(),
        Err(_) => format!(
            "{url} — unreachable (no response within {}ms)",
            PROBE_TIMEOUT.as_millis()
        ),
    };
    VoiceProbe {
        backend: backend_id.to_string(),
        detail,
    }
}

/// Shorter than the backend's own `timeout_ms`, which is sized for synthesis:
/// listing voices runs no model, so a server this slow to answer is broken.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(5_000);

/// One line about the configured backend's server, or `None` when it has no
/// server to probe.
async fn probe_configured_backend(backends: &Backends, backend_id: &str) -> Option<VoiceProbe> {
    if let Some(voicebox) = backends.voicebox(backend_id) {
        return Some(probe_voicebox(voicebox, backend_id).await);
    }
    let kokoro = backends.kokoro(backend_id)?;
    let url = kokoro.base_url().to_string();
    let detail = match tokio::time::timeout(PROBE_TIMEOUT, kokoro.voices()).await {
        // The model beside the address: docs/design.md#voice-cache.
        Ok(Ok(voices)) => format!(
            "{url} — reachable, model {}, {} voices",
            teleprompt_voice::VoiceBackend::capabilities(kokoro.as_ref()).version,
            voices.len()
        ),
        // In the backend's own words, which name the address: a refused
        // connection, a 500 and a bad voice list are different fixes.
        Ok(Err(e)) => e.to_string(),
        Err(_) => format!(
            "{url} — unreachable (no response within {}ms)",
            PROBE_TIMEOUT.as_millis()
        ),
    };
    Some(VoiceProbe {
        backend: backend_id.to_string(),
        detail,
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
            "  capture          {}\n",
            if self.capture_backends.is_empty() {
                "none — every scene renders as a slate".to_string()
            } else {
                self.capture_backends
                    .iter()
                    .map(|b| match &b.unavailable {
                        None => format!("{} ({})", b.id, b.adapter),
                        Some(why) => format!(
                            "{} ({}) — {why}; `{}`",
                            b.id,
                            b.adapter,
                            b.fix.as_deref().unwrap_or("teleprompt setup")
                        ),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ));
        out.push_str(&format!(
            "  voice backends   {}\n",
            self.voice_backends.join(", ")
        ));
        if let Some(p) = &self.voice_probe {
            out.push_str(&format!(
                "  voice {:<10} {}\n",
                p.backend,
                p.detail.as_str()
            ));
        }
        out.push_str(&format!(
            "  manifest         narration v{}\n",
            self.manifest_version
        ));
        out.push_str(&format!(
            "  cache            {}/voice — {} entries, {} bytes\n",
            self.cache_root, self.cache_entries, self.cache_bytes
        ));
        out.push_str(&format!(
            "  cache            {}/compose — {} entries, {} bytes\n",
            self.cache_root, self.compose_entries, self.compose_bytes
        ));
        out.push_str(&format!(
            "  ffmpeg           {}\n",
            self.ffmpeg.as_deref().unwrap_or(
                "not found — `build` cannot render without it; `teleprompt setup ffmpeg`"
            )
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

/// What this build can put on screen, and why it cannot where it cannot.
fn capture_backends() -> Vec<CaptureBackendStatus> {
    let registry = crate::scene::captures();
    registry
        .backends()
        .map(|b| {
            let unavailable = b.unavailable();
            CaptureBackendStatus {
                id: b.adapter().to_string(),
                adapter: b.adapter().to_string(),
                fix: unavailable
                    .as_ref()
                    .map(|_| format!("teleprompt setup {}", b.adapter())),
                unavailable,
            }
        })
        .collect()
}
