//! `capture`: running the scenes, so a build shows what its tapes do.
//!
//! Stage 5, between the manifest and the render. The unit of work is the
//! session, not the shot, because each shot opens on the screen the one
//! before it left (docs/design.md#capture).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use teleprompt_core::ShotId;
use teleprompt_manifest::NarrationManifest;
use teleprompt_pipeline::compile::ShotSource;
use teleprompt_scene::capture::{sessions, CaptureBackend, Frame, PlannedShot, Progress, Session};
use teleprompt_scene::{ScenePlugin, ScenePlugins};
use teleprompt_script::config::SceneConfig;

use crate::build::FrameOverride;
use crate::clips::ClipStore;
use crate::dub::{Dubbed, Written};
use teleprompt_core::Reporter;

/// The shots of a published manifest, as the planner needs them.
///
/// Built from the manifest, not the in-process timeline, for the reason
/// `build` is (docs/design.md#rendering).
fn cues_of(
    manifest: &NarrationManifest,
    shots: &BTreeMap<ShotId, ShotSource>,
    scenes: &BTreeMap<String, SceneConfig>,
) -> Vec<PlannedShot> {
    manifest
        .shots
        .iter()
        .map(|item| PlannedShot {
            id: item.shot.clone(),
            scene: item.scene.clone(),
            plugin: item.plugin.clone(),
            session: item.session.clone(),
            key: item.capture_key,
            source: shots
                .get(&item.shot)
                .map(|s| s.source.clone())
                .unwrap_or_default(),
            duration_ms: item.duration_ms.ms(),
            settings: scenes
                .get(&item.scene)
                .map(|s| flatten(&s.settings))
                .unwrap_or_default(),
        })
        .collect()
}

/// A scene's settings as strings.
///
/// Non-scalar values are left out rather than rendered as YAML, except one
/// level of nesting, flattened with a dot, so `env:` can be a map:
///
/// ```yaml
/// scene:
///   terminal:
///     env:
///       PATH: target/release:/usr/bin
/// ```
fn flatten(settings: &BTreeMap<String, serde_json::Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (key, value) in settings {
        match value {
            serde_json::Value::Object(map) => {
                for (name, value) in map {
                    if let Some(text) = scalar(value) {
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

fn scalar(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[derive(Debug, Serialize)]
pub struct CaptureReport {
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

impl Dubbed<Written> {
    /// Records whatever the shots this dub scheduled are missing from
    /// `clips`, with `plugins`, at the script's `output:` frame with
    /// `frame`'s overrides.
    ///
    /// Infallible by design: a scene nothing here can record, or a backend
    /// that fails, is a warning and slates. The timing is still real, and a
    /// video with a hole in it is more use than no video.
    pub fn capture(
        &self,
        plugins: &ScenePlugins,
        clips: &ClipStore,
        frame: FrameOverride,
        reporter: &dyn Reporter,
    ) -> CaptureReport {
        let frame = frame.frame(&self.output);
        let on_progress = &mut |p| reporter.progress(captured_shot(p));
        let shots = cues_of(&self.manifest, &self.shots, &self.scenes);
        let mut planned = sessions(&shots, &|key| clips.has(key));
        for session in &mut planned {
            if let Some(scene) = self.scenes.get(&session.scene) {
                session.root = scene.root.clone();
            }
        }

        let mut warnings = Vec::new();
        let mut captured = 0usize;
        let mut ran = 0usize;
        let mut uncaptured = 0usize;

        for session in &planned {
            let backend = plugins.get(&session.plugin).map(ScenePlugin::capture);
            match record(backend, session, &frame, clips, on_progress) {
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
            clips_dir: clips.dir().to_path_buf(),
            sessions: ran,
            captured,
            reused: total.saturating_sub(captured + uncaptured),
            uncaptured,
            warnings,
        }
    }
}

/// A shot's capture progress, as a [`Reporter`] takes it.
fn captured_shot(p: Progress) -> teleprompt_core::Progress {
    teleprompt_core::Progress::Capture {
        done: p.done,
        of: p.of,
        scene: p.scene,
        shot: p.shot,
    }
}

/// Record a session with `backend`, its plugin's, or say why it will be
/// slates.
fn record(
    backend: Option<&dyn CaptureBackend>,
    session: &Session,
    frame: &Frame,
    clips: &ClipStore,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<usize, String> {
    let Some(backend) = backend else {
        return Err(format!(
            "nothing in this build can record `{}` scenes (plugin `{}`), so \
             its {} shot(s) will render as slates",
            session.scene,
            session.plugin,
            session.wanted()
        ));
    };
    if let Some(reason) = backend.unavailable() {
        // A tool that is not installed is for `setup` to say how to install.
        let fix = teleprompt_core::tool::programs_in(&reason)
            .map(|_| {
                format!(
                    "; `teleprompt setup {}` says how to install it",
                    session.plugin
                )
            })
            .unwrap_or_default();
        return Err(format!(
            "`{}` cannot record `{}` scenes here: {reason}{fix}; its {} shot(s) \
             will render as slates",
            session.plugin,
            session.scene,
            session.wanted()
        ));
    }
    clips
        .record(|staging| {
            backend
                .capture(session, frame, staging, on_progress)
                .map_err(|e| e.to_string())
        })
        .map_err(|e| {
            format!(
                "recording `{}` failed, so its {} shot(s) will render as \
                 slates: {e}",
                session.scene,
                session.wanted()
            )
        })
}
