use std::path::{Path, PathBuf};

use teleprompt_compile::manifest::{self, AudioInfo, NarrationManifest, MANIFEST_VERSION};
use teleprompt_compile::manifest_diff::{self, ManifestDiff};
use teleprompt_voice::{wav, NullVoice, SynthRequest, VoiceBackend, NULL_SAMPLE_RATE};

use crate::cmd::check::compile_script;
use crate::project::Project;

pub struct DubOutput {
    pub manifest: NarrationManifest,
    pub written: Vec<PathBuf>,
    pub warnings: Vec<String>,
    /// `Some` only under `--check`. `None` means nothing was compared.
    pub drift: Option<ManifestDiff>,
}

/// Distinguishes a script that fails validation (exit 2) from a runtime
/// failure such as an unreadable committed manifest or a write error (exit
/// 1). A flat `Vec<String>` cannot carry that distinction, so `run_dub`
/// returns this instead of reusing `compile_script`'s error type directly.
pub enum DubError {
    Validation(Vec<String>),
    Runtime(String),
}

/// Where this locale's self-contained directory lives.
fn locale_dir(out_root: &Path, locale: &str) -> PathBuf {
    out_root.join(locale)
}

pub fn manifest_path(out_root: &Path, locale: &str) -> PathBuf {
    locale_dir(out_root, locale).join("narration.json")
}

/// Read a committed manifest, refusing a version this build does not
/// understand rather than letting serde fill in defaults and produce a
/// confident, wrong diff.
fn read_committed(path: &Path) -> Result<Option<NarrationManifest>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let probe: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let version = probe.get("manifest_version").and_then(|v| v.as_u64());
    if version != Some(MANIFEST_VERSION as u64) {
        return Err(format!(
            "{}: manifest_version {} is not supported (this build writes {})",
            path.display(),
            version
                .map(|v| v.to_string())
                .unwrap_or_else(|| "missing".into()),
            MANIFEST_VERSION,
        ));
    }
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

pub fn run_dub(
    project: &Project,
    script: &Path,
    locale: &str,
    out_root: &Path,
    check_only: bool,
) -> Result<DubOutput, DubError> {
    let compiled = compile_script(project, script, locale).map_err(DubError::Validation)?;

    // This backend is separate from the `NullVoice` `compile_script`
    // constructs internally for duration estimation — a deliberate
    // duplication until a real backend exists. `render_pcm` is only needed
    // here, so `compile_script`'s signature (and `run_check`/`run_plan`/
    // `run_diff`, which depend on it) stays untouched; when a real backend
    // lands, both call sites will select it together.
    let voice = NullVoice::default();

    // Audio is rendered into memory before anything is written to disk, so
    // a synthesis failure cannot leave a half-populated output directory
    // behind.
    let mut audio: Vec<(String, Vec<u8>)> = Vec::new();
    let mut sample_rate = NULL_SAMPLE_RATE;
    let mut channels = 1u16;

    for detail in &compiled.narration {
        let req = SynthRequest {
            text: detail.text.clone(),
            locale: locale.to_string(),
            voice: None,
            speed: 1.0,
        };
        match voice.render_pcm(&req) {
            Ok(Some(pcm)) => {
                sample_rate = pcm.sample_rate;
                channels = pcm.channels;
                audio.push((detail.segment_id.clone(), wav::encode(&pcm)));
            }
            Ok(None) => {
                return Err(DubError::Runtime(format!(
                    "backend `{}` produces no audio; `dub` needs a backend that can render",
                    voice.id()
                )))
            }
            Err(e) => {
                return Err(DubError::Runtime(format!(
                    "segment `{}`: {e}",
                    detail.segment_id
                )))
            }
        }
    }

    let built = manifest::build(
        &compiled.timeline,
        &compiled.chapters,
        &compiled.narration,
        AudioInfo {
            format: "wav".to_string(),
            sample_rate,
            channels,
        },
    );

    if check_only {
        let committed =
            read_committed(&manifest_path(out_root, locale)).map_err(DubError::Runtime)?;
        let drift = match committed {
            Some(before) => manifest_diff::diff(&before, &built),
            // No manifest at all is maximal drift: everything is new.
            None => manifest_diff::diff(
                &NarrationManifest {
                    segments: Vec::new(),
                    chapters: Vec::new(),
                    duration_ms: 0,
                    ..built.clone()
                },
                &built,
            ),
        };
        return Ok(DubOutput {
            manifest: built,
            written: Vec::new(),
            warnings: compiled.warnings,
            drift: Some(drift),
        });
    }

    let dir = locale_dir(out_root, locale);
    let audio_dir = dir.join("audio");
    std::fs::create_dir_all(&audio_dir)
        .map_err(|e| DubError::Runtime(format!("cannot create {}: {e}", audio_dir.display())))?;

    let mut written = Vec::new();
    for (segment_id, bytes) in &audio {
        let path = dir.join(manifest::audio_path(segment_id, "wav"));
        std::fs::write(&path, bytes)
            .map_err(|e| DubError::Runtime(format!("cannot write {}: {e}", path.display())))?;
        written.push(path);
    }

    let path = manifest_path(out_root, locale);
    let json = serde_json::to_string_pretty(&built)
        .map_err(|e| DubError::Runtime(format!("cannot serialize manifest: {e}")))?;
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| DubError::Runtime(format!("cannot write {}: {e}", path.display())))?;
    written.push(path);

    Ok(DubOutput {
        manifest: built,
        written,
        warnings: compiled.warnings,
        drift: None,
    })
}

pub fn render_dub(out: &DubOutput) -> String {
    let mut s = format!(
        "{} ({}) — {} segment(s), {:.1}s\n",
        out.manifest.script,
        out.manifest.locale,
        out.manifest.segments.len(),
        out.manifest.duration_ms as f64 / 1000.0,
    );
    for path in &out.written {
        s.push_str(&format!("  wrote {}\n", path.display()));
    }
    s
}
