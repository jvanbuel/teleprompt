//! Images, clips and title cards, drawn with nothing but ffmpeg.
//!
//! [`scene`] reads a block of directives and [`capture`] turns each into a
//! clip. The one scene with no existing tool worth adopting, so it has a
//! handful of directives of its own.

pub mod capture;
pub mod scene;

pub use capture::MediaRender;
pub use scene::MediaScene;

use std::path::PathBuf;
use teleprompt_scene::SceneSettings;

/// What both halves read of a media scene's settings: the files its
/// capture key covers are the files its capture draws from.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    /// `dir`: where its images and clips are, by default `media`.
    pub dir: PathBuf,
}

impl Settings {
    pub(crate) fn of(scene: &impl SceneSettings) -> Self {
        Self {
            dir: scene.path_or("dir", "media"),
        }
    }
}

/// The plugin, to register.
pub fn plugin() -> teleprompt_scene::ScenePlugin {
    teleprompt_scene::ScenePlugin::new(MediaScene, MediaRender::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use teleprompt_scene::capture::{session_settings, Session};
    use teleprompt_scene::core::config::SceneConfig;

    /// A scene configured with `settings`, and the session capture makes of it.
    fn halves(settings: serde_json::Value) -> (SceneConfig, Session) {
        let scene = SceneConfig {
            plugin: "media".into(),
            settings: serde_json::from_value(settings).unwrap(),
            root: "/project".into(),
        };
        let session = Session {
            scene: "s".into(),
            plugin: "media".into(),
            name: None,
            settings: session_settings(&scene.settings),
            root: scene.root.clone(),
            shots: Vec::new(),
        };
        (scene, session)
    }

    #[test]
    fn compile_and_capture_read_the_same_media_directory() {
        for settings in [serde_json::json!({}), serde_json::json!({"dir": "assets"})] {
            let (scene, session) = halves(settings);
            assert_eq!(Settings::of(&scene), Settings::of(&session));
        }
    }

    #[test]
    fn the_capture_key_covers_the_image_capture_draws() {
        use teleprompt_scene::SceneCompiler;
        let (scene, session) = halves(serde_json::json!({"dir": "assets"}));
        let inputs = MediaScene.shot_inputs(&scene, "image src=logo.png");
        assert_eq!(inputs, [Settings::of(&session).dir.join("logo.png")]);
    }
}
