//! `capture`: running the scenes, so a build shows what its tapes do.
//!
//! Stage 5, between the manifest and the render. The unit of work is the
//! session, not the shot, because each shot opens on the screen the one
//! before it left (docs/design.md#capture).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_compile::ShotSource;
use teleprompt_core::config::SceneConfig;
use teleprompt_core::ShotId;
use teleprompt_manifest::NarrationManifest;
use teleprompt_scene::capture::{
    sessions, CaptureBackend, Frame, PlannedShot, Progress, Session, WorkDir,
};
use teleprompt_scene::{ScenePlugin, ScenePlugins};

use crate::project::{CacheDir, Clips};

/// The shots of a published manifest, as the planner needs them.
///
/// Built from the manifest, not the in-process timeline, for the reason
/// `build` is (docs/design.md#rendering).
pub(crate) fn cues_of(
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

/// The scene plugins, recording into a clip directory at one frame size.
pub struct Scenes<'a> {
    plugins: &'a ScenePlugins,
    clips_dir: &'a CacheDir<Clips>,
    frame: Frame,
}

impl<'a> Scenes<'a> {
    pub fn new(plugins: &'a ScenePlugins, clips_dir: &'a CacheDir<Clips>, frame: Frame) -> Self {
        Self {
            plugins,
            clips_dir,
            frame,
        }
    }

    /// Records whatever the shots `dub` scheduled are missing from the clip
    /// directory.
    ///
    /// Infallible by design: a scene nothing here can record, or a backend
    /// that fails, is a warning and slates. The timing is still real, and a
    /// video with a hole in it is more use than no video.
    pub fn capture(
        &self,
        dubbed: &crate::dub::Dubbed<crate::dub::Written>,
        on_progress: &mut dyn FnMut(Progress),
    ) -> CaptureReport {
        let clips_dir = self.clips_dir;
        let scenes = &dubbed.scenes;
        let shots = cues_of(&dubbed.manifest, &dubbed.shots, scenes);
        let have = |key: &teleprompt_core::Hash| clips_dir.join(format!("{key}.mp4")).is_file();
        let mut planned = sessions(&shots, &have);
        for session in &mut planned {
            if let Some(scene) = scenes.get(&session.scene) {
                session.root = scene.root.clone();
            }
        }

        let mut warnings = Vec::new();
        let mut captured = 0usize;
        let mut ran = 0usize;
        let mut uncaptured = 0usize;

        for session in &planned {
            let backend = self.plugins.get(&session.plugin).map(ScenePlugin::capture);
            match record(backend, session, &self.frame, clips_dir, on_progress) {
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
            clips_dir: clips_dir.to_path_buf(),
            sessions: ran,
            captured,
            reused: total.saturating_sub(captured + uncaptured),
            uncaptured,
            warnings,
        }
    }
}

/// Record a session with `backend`, its plugin's, or say why it will be
/// slates.
fn record(
    backend: Option<&dyn CaptureBackend>,
    session: &Session,
    frame: &Frame,
    clips_dir: &Path,
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
        return Err(format!(
            "`{}` cannot record `{}` scenes here: {reason}; its {} shot(s) \
             will render as slates",
            session.plugin,
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

#[cfg(test)]
mod tests {
    use super::*;
    use teleprompt_core::Hash;
    use teleprompt_scene::capture::{CaptureBackend, CaptureError, Clip, SessionShot};

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
            plugin: "writes".into(),
            name: None,
            settings: BTreeMap::new(),
            root: Default::default(),
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
        let s = session();
        assert!(record(
            Some(&Writes { then_fail: true }),
            &s,
            &frame(),
            &dir,
            &mut |_| {}
        )
        .is_err());
        let clip = dir.join(format!("{}.mp4", s.shots[0].key));
        assert!(!clip.exists(), "a failed session left {}", clip.display());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_finished_session_files_its_clips_under_their_keys() {
        let dir = clips_dir("ok");
        let s = session();
        assert_eq!(
            record(
                Some(&Writes { then_fail: false }),
                &s,
                &frame(),
                &dir,
                &mut |_| {}
            ),
            Ok(1)
        );
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
