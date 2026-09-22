//! `capture`: running the scenes, so a build shows what its tapes do.
//!
//! Stage 5. It sits between the manifest and the render, and it is what
//! turns a correctly-paced video of slates into a video.
//!
//! The work is organised by session rather than by beat, because a scene
//! *is* a session: the beats of a walkthrough continue one another, so a
//! backend is handed a run of a scene and told which of its steps to keep.
//! A step whose clip is already cached still runs — the step after it
//! opens on the screen it leaves behind — which is why the unit here is
//! not the beat.

use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_capture::{sessions, Beat, CaptureError, CaptureRegistry, Frame, Progress, Session};
use teleprompt_compile::manifest::NarrationManifest;
use teleprompt_compile::SpanSource;

/// The beats of a published manifest, as the planner needs them.
///
/// Built from the manifest rather than from the in-process timeline for
/// the reason the renderer is: two timing paths drift, and a clip captured
/// against a length nothing published is a clip that does not fit.
pub fn beats_of(manifest: &NarrationManifest, spans: &[SpanSource]) -> Vec<Beat> {
    manifest
        .beats
        .iter()
        .map(|beat| Beat {
            span: beat.span.clone(),
            scene: beat.scene.clone(),
            adapter: beat.adapter.clone(),
            session: beat.session.clone(),
            key: beat.capture_key,
            source: spans
                .iter()
                .find(|s| s.id == beat.span)
                .map(|s| s.source.clone())
                .unwrap_or_default(),
            duration_ms: beat.duration_ms,
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct CaptureReport {
    pub ok: bool,
    pub clips_dir: PathBuf,
    /// Sessions that had something to record. A warm project has none.
    pub sessions: usize,
    pub captured: usize,
    /// Beats whose clip was already in hand.
    pub reused: usize,
    /// Beats nothing on this machine can record, which `build` will render
    /// as slates.
    pub uncaptured: usize,
    pub warnings: Vec<String>,
}

impl CaptureReport {
    pub fn render(&self) -> String {
        format!(
            "  {}\n  {} session(s), {} captured, {} reused, {} uncaptured\n",
            self.clips_dir.display(),
            self.sessions,
            self.captured,
            self.reused,
            self.uncaptured,
        )
    }
}

/// Record whatever is missing from `clips_dir`.
///
/// Never an error for a scene this build cannot record: a machine with no
/// terminal backend is a machine that gets slates and a warning, which is
/// what `build` already does for a scene nothing has captured. An error
/// here is a backend that was asked to do its job and failed at it.
pub fn run_capture(
    manifest: &NarrationManifest,
    spans: &[SpanSource],
    registry: &CaptureRegistry,
    clips_dir: &Path,
    frame: Frame,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<CaptureReport, CaptureError> {
    let beats = beats_of(manifest, spans);
    let have = |key: &teleprompt_core::Hash| clips_dir.join(format!("{key}.mp4")).is_file();
    let planned = sessions(&beats, &have);

    let mut warnings = Vec::new();
    let mut captured = 0usize;
    let mut ran = 0usize;
    let mut uncaptured = 0usize;

    for session in &planned {
        match usable(registry, session) {
            Err(why) => {
                uncaptured += session.wanted();
                warnings.push(why);
            }
            Ok(backend) => {
                let shots = backend.capture(session, &frame, clips_dir, on_progress)?;
                captured += shots.len();
                ran += 1;
            }
        }
    }

    let total = beats.iter().filter(|b| b.scene != "pause").count();
    Ok(CaptureReport {
        ok: true,
        clips_dir: clips_dir.to_path_buf(),
        sessions: ran,
        captured,
        reused: total.saturating_sub(captured + uncaptured),
        uncaptured,
        warnings,
    })
}

/// The backend for a session, or the reason there is not one — in the
/// words an author can act on, naming the scene rather than the adapter,
/// because the scene is what they wrote.
fn usable<'a>(
    registry: &'a CaptureRegistry,
    session: &Session,
) -> Result<&'a dyn teleprompt_capture::CaptureBackend, String> {
    let Some(backend) = registry.for_adapter(&session.adapter) else {
        return Err(format!(
            "nothing in this build can record `{}` scenes (adapter `{}`), so \
             its {} beat(s) will render as slates",
            session.scene,
            session.adapter,
            session.wanted()
        ));
    };
    match backend.unavailable() {
        Some(reason) => Err(format!(
            "`{}` cannot record `{}` scenes here: {reason}; its {} beat(s) \
             will render as slates",
            backend.id(),
            session.scene,
            session.wanted()
        )),
        None => Ok(backend),
    }
}

/// The backends this build ships.
pub fn registry() -> CaptureRegistry {
    CaptureRegistry::new().with(Box::new(teleprompt_capture::mock::MockCapture::default()))
}
