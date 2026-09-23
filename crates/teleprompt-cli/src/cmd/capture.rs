//! `capture`: running the scenes, so a build shows what its tapes do.
//!
//! Stage 5. It sits between the manifest and the render, and it is what
//! turns a correctly-paced video of slates into a video.
//!
//! The work is organised by session rather than by shot, because a scene
//! *is* a session: the shots of a walkthrough continue one another, so a
//! backend is handed a run of a scene and told which of its shots to keep.
//! A shot whose clip is already cached still runs — the shot after it
//! opens on the screen it leaves behind — which is why the unit here is
//! not the shot.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_capture::{sessions, CaptureRegistry, Frame, Progress, Session, Shot};
use teleprompt_compile::manifest::NarrationManifest;
use teleprompt_compile::ShotSource;
use teleprompt_core::config::SceneConfig;

/// The shots of a published manifest, as the planner needs them.
///
/// Built from the manifest rather than from the in-process timeline for
/// the reason the renderer is: two timing paths drift, and a clip captured
/// against a length nothing published is a clip that does not fit.
pub fn cues_of(
    manifest: &NarrationManifest,
    shots: &[ShotSource],
    scenes: &BTreeMap<String, SceneConfig>,
) -> Vec<Shot> {
    manifest
        .shots
        .iter()
        .map(|item| Shot {
            id: item.shot.clone(),
            scene: item.scene.clone(),
            adapter: item.adapter.clone(),
            session: item.session.clone(),
            key: item.capture_key,
            source: shots
                .iter()
                .find(|s| s.id == item.shot)
                .map(|s| s.source.clone())
                .unwrap_or_default(),
            duration_ms: item.duration_ms,
            settings: scenes
                .get(&item.scene)
                .map(|s| flatten(&s.settings))
                .unwrap_or_default(),
        })
        .collect()
}

/// A scene's settings as strings.
///
/// A backend reads the handful of keys it understands — how many columns,
/// which shell, what to put in the environment — and none of them are
/// structured. Anything that is not a scalar is left out rather than
/// rendered as YAML, because a backend that cannot read it would be
/// reading a syntax, not a value.
///
/// One level of nesting survives, flattened with a dot, so that `env:` can
/// be written as the map it is:
///
/// ```yaml
/// scene:
///   terminal:
///     env:
///       PATH: target/release:/usr/bin
/// ```
fn flatten(settings: &BTreeMap<String, serde_yaml::Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (key, value) in settings {
        match value {
            serde_yaml::Value::Mapping(map) => {
                for (inner, value) in map {
                    if let (Some(name), Some(text)) = (inner.as_str(), scalar(value)) {
                        out.insert(format!("{key}.{name}"), text);
                    }
                }
            }
            other => {
                if let Some(text) = scalar(other) {
                    out.insert(key.clone(), text);
                }
            }
        }
    }
    out
}

fn scalar(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[derive(Debug, Serialize)]
pub struct CaptureReport {
    pub ok: bool,
    pub clips_dir: PathBuf,
    /// Sessions that had something to record. A warm project has none.
    pub sessions: usize,
    pub captured: usize,
    /// Cues whose clip was already in hand.
    pub reused: usize,
    /// Cues nothing on this machine can record, which `build` will render
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
/// Infallible by design. A scene nothing here can record is slates and a
/// warning — the timing is still real, and a video with a hole in it is
/// more use than no video — and so is a backend that was asked and failed,
/// because there may be another one that manages.
pub fn run_capture(
    manifest: &NarrationManifest,
    shots: &[ShotSource],
    scenes: &BTreeMap<String, SceneConfig>,
    registry: &CaptureRegistry,
    clips_dir: &Path,
    frame: Frame,
    on_progress: &mut dyn FnMut(Progress),
) -> CaptureReport {
    let shots = cues_of(manifest, shots, scenes);
    let have = |key: &teleprompt_core::Hash| clips_dir.join(format!("{key}.mp4")).is_file();
    let planned = sessions(&shots, &have);

    let mut warnings = Vec::new();
    let mut captured = 0usize;
    let mut ran = 0usize;
    let mut uncaptured = 0usize;

    for session in &planned {
        match record(registry, session, &frame, clips_dir, on_progress) {
            Ok(clips) => {
                captured += clips;
                ran += 1;
            }
            Err(why) => {
                uncaptured += session.wanted();
                warnings.push(why);
            }
        }
    }

    let total = shots.iter().filter(|b| b.scene != "pause").count();
    CaptureReport {
        ok: true,
        clips_dir: clips_dir.to_path_buf(),
        sessions: ran,
        captured,
        reused: total.saturating_sub(captured + uncaptured),
        uncaptured,
        warnings,
    }
}

/// Record a session, or say why it will be slates.
fn record(
    registry: &CaptureRegistry,
    session: &Session,
    frame: &Frame,
    clips_dir: &Path,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<usize, String> {
    let Some(backend) = registry.for_adapter(&session.adapter) else {
        return Err(format!(
            "nothing in this build can record `{}` scenes (adapter `{}`), so \
             its {} shot(s) will render as slates",
            session.scene,
            session.adapter,
            session.wanted()
        ));
    };
    if let Some(reason) = backend.unavailable() {
        return Err(format!(
            "`{}` cannot record `{}` scenes here: {reason}; its {} shot(s) \
             will render as slates",
            backend.id(),
            session.scene,
            session.wanted()
        ));
    }
    backend
        .capture(session, frame, clips_dir, on_progress)
        .map(|clips| clips.len())
        .map_err(|e| {
            format!(
                "recording `{}` failed, so its {} shot(s) will render as \
                 slates: {e}",
                session.scene,
                session.wanted()
            )
        })
}

/// The backends this build ships: one per scene adapter.
///
/// `vhs` renders terminal scenes. teleprompt already re-times every tape
/// to its slot, so handing that tape to the program built to record tapes
/// is the whole job — and it draws the window, the theme and the padding
/// that nothing else would.
pub fn registry() -> CaptureRegistry {
    CaptureRegistry::new()
        .with(Box::new(teleprompt_vhs::VhsRender::default()))
        .with(Box::new(teleprompt_playwright::PlaywrightRender::default()))
        .with(Box::new(teleprompt_remotion::RemotionRender::default()))
        .with(Box::new(teleprompt_slidev::SlidevRender::default()))
        .with(Box::new(teleprompt_capture::mock::MockCapture::default()))
}
