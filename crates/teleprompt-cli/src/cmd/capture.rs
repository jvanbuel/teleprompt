//! `capture`: running the scenes, so a build shows what its tapes do.
//!
//! Stage 5, between the manifest and the render. The unit of work is the
//! session, not the shot, because each shot opens on the screen the one
//! before it left (docs/design.md#capture).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_capture::{
    sessions, CaptureRegistry, Frame, PlannedShot, Progress, Session, WorkDir,
};
use teleprompt_compile::ShotSource;
use teleprompt_core::config::SceneConfig;
use teleprompt_manifest::NarrationManifest;

/// The shots of a published manifest, as the planner needs them.
///
/// Built from the manifest, not the in-process timeline, for the reason
/// `build` is (docs/design.md#rendering).
pub(crate) fn cues_of(
    manifest: &NarrationManifest,
    shots: &[ShotSource],
    scenes: &BTreeMap<String, SceneConfig>,
) -> Vec<PlannedShot> {
    manifest
        .shots
        .iter()
        .map(|item| PlannedShot {
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
/// Infallible by design: a scene nothing here can record, or a backend
/// that fails, is a warning and slates. The timing is still real, and a
/// video with a hole in it is more use than no video.
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
            session.adapter,
            session.scene,
            session.wanted()
        ));
    }
    // The backend writes into a staging directory beside the cache, and a
    // clip is renamed into place only once the session has succeeded: a
    // clip in the cache is taken as captured, so a partial one must never
    // land there. Dropping the staging directory discards a failed run.
    let failed = |e: String| {
        format!(
            "recording `{}` failed, so its {} shot(s) will render as \
             slates: {e}",
            session.scene,
            session.wanted()
        )
    };
    let staging = std::fs::create_dir_all(clips_dir)
        .and_then(|()| WorkDir::create(clips_dir, "staging"))
        .map_err(|e| failed(format!("{}: {e}", clips_dir.display())))?;
    let clips = backend
        .capture(session, frame, &staging, on_progress)
        .map_err(|e| failed(e.to_string()))?;
    for clip in &clips {
        let into = clips_dir.join(format!("{}.mp4", clip.key));
        std::fs::rename(&clip.path, &into)
            .map_err(|e| failed(format!("cannot file {}: {e}", into.display())))?;
    }
    Ok(clips.len())
}

/// `teleprompt capture`: dubs the script, then records whatever its shots
/// are missing into the build's clip directory.
///
/// It dubs first rather than read a manifest on disk that may be stale
/// (docs/design.md#rendering). `size` and `fps` override the script's own
/// `output:` frame.
pub async fn capture_script(
    project: &crate::project::Project,
    script: &Path,
    locale: &str,
    size: Option<(u32, u32)>,
    fps: Option<u32>,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<CaptureReport, crate::cmd::dub::DubError> {
    let options = crate::cmd::build::BuildOptions::defaults(project, script, locale);
    let dubbed =
        crate::cmd::dub::run_dub(project, script, locale, &options.narration_root, false).await?;
    let (width, height) = size.unwrap_or(dubbed.output.resolution);
    let frame = Frame {
        width,
        height,
        fps: fps.unwrap_or(dubbed.output.fps),
    };
    Ok(run_capture(
        &dubbed.manifest,
        &dubbed.shots,
        &dubbed.scenes,
        &crate::scene::captures(),
        &options.clips_dir,
        frame,
        on_progress,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use teleprompt_capture::{CaptureBackend, CaptureError, Clip, SessionShot};
    use teleprompt_core::Hash;

    /// Writes each wanted shot's clip, then fails if told to.
    struct Writes {
        then_fail: bool,
    }

    impl CaptureBackend for Writes {
        fn capture(
            &self,
            session: &Session,
            _frame: &Frame,
            out_dir: &Path,
            _on_progress: &mut dyn FnMut(Progress),
        ) -> Result<Vec<Clip>, CaptureError> {
            std::fs::create_dir_all(out_dir).unwrap();
            let mut clips = Vec::new();
            for shot in session.shots.iter().filter(|s| s.wanted) {
                let path = out_dir.join(format!("{}.mp4", shot.key));
                std::fs::write(&path, b"half a clip").unwrap();
                clips.push(Clip {
                    key: shot.key,
                    path,
                });
            }
            if self.then_fail {
                return Err(CaptureError::Failed {
                    backend: "writes".into(),
                    shot: session.shots[0].id.clone(),
                    reason: "interrupted".into(),
                });
            }
            Ok(clips)
        }
    }

    fn session() -> Session {
        Session {
            scene: "terminal".into(),
            adapter: "writes".into(),
            name: None,
            settings: BTreeMap::new(),
            shots: vec![SessionShot {
                id: "a#0".into(),
                key: Hash::of(b"a#0"),
                source: String::new(),
                duration_ms: 1000,
                wanted: true,
            }],
        }
    }

    fn clips_dir(name: &str) -> teleprompt_testkit::TestDir {
        teleprompt_testkit::test_dir(&format!("record-{name}"))
    }

    fn frame() -> Frame {
        Frame {
            width: 64,
            height: 36,
            fps: 30,
        }
    }

    /// A clip is in the cache only once its backend has finished: a failed
    /// session leaves nothing behind that a later build would take as
    /// captured.
    #[test]
    fn a_failed_session_leaves_no_clip_in_the_cache() {
        let dir = clips_dir("fail");
        let registry = CaptureRegistry::new().with("writes", Box::new(Writes { then_fail: true }));
        let s = session();
        assert!(record(&registry, &s, &frame(), &dir, &mut |_| {}).is_err());
        let clip = dir.join(format!("{}.mp4", s.shots[0].key));
        assert!(!clip.exists(), "a failed session left {}", clip.display());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_finished_session_files_its_clips_under_their_keys() {
        let dir = clips_dir("ok");
        let registry = CaptureRegistry::new().with("writes", Box::new(Writes { then_fail: false }));
        let s = session();
        assert_eq!(record(&registry, &s, &frame(), &dir, &mut |_| {}), Ok(1));
        assert!(dir.join(format!("{}.mp4", s.shots[0].key)).is_file());
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "staging left behind: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
