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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_capture::{sessions, Beat, CaptureRegistry, Frame, Progress, Session};
use teleprompt_compile::manifest::NarrationManifest;
use teleprompt_compile::SpanSource;
use teleprompt_core::config::SceneConfig;

/// The beats of a published manifest, as the planner needs them.
///
/// Built from the manifest rather than from the in-process timeline for
/// the reason the renderer is: two timing paths drift, and a clip captured
/// against a length nothing published is a clip that does not fit.
pub fn beats_of(
    manifest: &NarrationManifest,
    spans: &[SpanSource],
    scenes: &BTreeMap<String, SceneConfig>,
) -> Vec<Beat> {
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
            settings: scenes
                .get(&beat.scene)
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
/// Infallible by design. A scene nothing here can record is slates and a
/// warning — the timing is still real, and a video with a hole in it is
/// more use than no video — and so is a backend that was asked and failed,
/// because there may be another one that manages.
pub fn run_capture(
    manifest: &NarrationManifest,
    spans: &[SpanSource],
    scenes: &BTreeMap<String, SceneConfig>,
    registry: &CaptureRegistry,
    clips_dir: &Path,
    frame: Frame,
    on_progress: &mut dyn FnMut(Progress),
) -> CaptureReport {
    let beats = beats_of(manifest, spans, scenes);
    let have = |key: &teleprompt_core::Hash| clips_dir.join(format!("{key}.mp4")).is_file();
    let planned = sessions(&beats, &have);

    let mut warnings = Vec::new();
    let mut captured = 0usize;
    let mut ran = 0usize;
    let mut uncaptured = 0usize;

    for session in &planned {
        match record(registry, session, &frame, clips_dir, on_progress) {
            Ok(shots) => {
                captured += shots;
                ran += 1;
            }
            Err(why) => {
                uncaptured += session.wanted();
                warnings.push(why);
            }
        }
    }

    let total = beats.iter().filter(|b| b.scene != "pause").count();
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

/// Record a session with the best backend that manages it.
///
/// Tried in order and fallen back on failure, because whether a backend
/// can work here is not answerable in advance: `vhs` can be installed,
/// pass every check, and record nothing. The failure is the test.
fn record(
    registry: &CaptureRegistry,
    session: &Session,
    frame: &Frame,
    clips_dir: &Path,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<usize, String> {
    let candidates = registry.candidates(&session.adapter);
    if candidates.is_empty() {
        return Err(format!(
            "nothing in this build can record `{}` scenes (adapter `{}`), so \
             its {} beat(s) will render as slates",
            session.scene,
            session.adapter,
            session.wanted()
        ));
    }

    let mut refused = Vec::new();
    for backend in candidates {
        if let Some(reason) = backend.unavailable() {
            refused.push(format!("`{}` {reason}", backend.id()));
            continue;
        }
        match backend.capture(session, frame, clips_dir, on_progress) {
            Ok(shots) => {
                if !refused.is_empty() {
                    // Worth saying: the scene was recorded, and not by the
                    // renderer that would have drawn it best.
                    eprintln!(
                        "warning: recorded `{}` with `{}` — {}",
                        session.scene,
                        backend.id(),
                        refused.join("; ")
                    );
                }
                return Ok(shots.len());
            }
            Err(e) => refused.push(format!("`{}` failed: {e}", backend.id())),
        }
    }

    Err(format!(
        "nothing could record `{}` here, so its {} beat(s) will render as \
         slates: {}",
        session.scene,
        session.wanted(),
        refused.join("; ")
    ))
}

/// The backends this build ships, best first.
///
/// `vhs` renders a terminal scene where it can: teleprompt already
/// re-times every tape to its slot, so handing that tape to the program
/// that was built to record tapes is the whole job, and it draws the
/// window, the theme and the padding that a plain terminal does not.
///
/// It needs `ttyd` and a browser, though, and where those are missing it
/// does not fail loudly — it has been seen to exit 0 having recorded
/// nothing. So the registry asks each backend whether it can work here and
/// takes the first that says yes; the pty renderer is the fallback for
/// everywhere else, which is most containers.
pub fn registry() -> CaptureRegistry {
    CaptureRegistry::new()
        .with(Box::new(
            teleprompt_capture_vhs::render::VhsRender::default(),
        ))
        .with(Box::new(teleprompt_capture_vhs::VhsCapture::default()))
        .with(Box::new(teleprompt_capture::mock::MockCapture::default()))
}
