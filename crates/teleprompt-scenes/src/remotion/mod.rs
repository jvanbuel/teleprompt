//! Motion graphics, drawn by an existing Remotion project.
//!
//! [`scene`] reads a block — which composition, with which props — and
//! [`capture`] asks the project's own Remotion install to render it at the
//! length the narration gives it. The reverse of
//! `docs/integrations/remotion.md`, where Remotion owns the whole video.

pub mod capture;
pub mod scene;
pub mod tools;

pub use capture::RemotionRender;
pub use scene::RemotionScene;

use std::path::PathBuf;
use teleprompt_scene::SceneSettings;

/// What both halves read of a Remotion scene's settings: the files its
/// capture key covers are the files its capture bundles.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    /// `project`: the Remotion project, by default the project itself.
    pub project: PathBuf,
    /// `entry`, within the project: by default `src/index.ts`.
    pub entry: PathBuf,
}

impl Settings {
    pub(crate) fn of(scene: &impl SceneSettings) -> Self {
        Self {
            project: scene.path_or("project", "."),
            entry: PathBuf::from(scene.value_or("entry", "src/index.ts")),
        }
    }
}

/// The plugin, to register.
pub fn plugin() -> teleprompt_scene::ScenePlugin {
    teleprompt_scene::ScenePlugin::new(RemotionScene, RemotionRender::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use teleprompt_scene::capture::{session_settings, Session};
    use teleprompt_scene::core::config::SceneConfig;

    /// A scene configured with `settings`, and the session capture makes of it.
    fn halves(settings: serde_json::Value) -> (SceneConfig, Session) {
        let scene = SceneConfig {
            plugin: "remotion".into(),
            settings: serde_json::from_value(settings).unwrap(),
            root: "/project".into(),
        };
        let session = Session {
            scene: "s".into(),
            plugin: "remotion".into(),
            name: None,
            settings: session_settings(&scene.settings),
            root: scene.root.clone(),
            shots: Vec::new(),
        };
        (scene, session)
    }

    #[test]
    fn compile_and_capture_read_the_same_project_and_entry() {
        for settings in [
            serde_json::json!({}),
            serde_json::json!({"project": "motion", "entry": "src/main.tsx"}),
        ] {
            let (scene, session) = halves(settings);
            assert_eq!(Settings::of(&scene), Settings::of(&session));
        }
    }

    #[test]
    fn the_capture_key_covers_the_entry_capture_bundles() {
        use teleprompt_scene::SceneCompiler;
        let (scene, session) =
            halves(serde_json::json!({"project": "motion", "entry": "src/main.tsx"}));
        let read = Settings::of(&session);
        let entry_dir = read
            .project
            .join(&read.entry)
            .parent()
            .unwrap()
            .to_path_buf();
        assert!(RemotionScene.inputs(&scene).contains(&entry_dir));
    }
}
