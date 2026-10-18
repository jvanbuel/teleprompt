//! Slides, from an existing Slidev deck.
//!
//! [`scene`] reads a block — which slide, after how many clicks — and
//! [`capture`] has the deck's own Slidev export those slides as stills.

pub mod capture;
pub mod scene;
pub mod tools;

pub use capture::SlidevRender;
pub use scene::SlidevScene;

use std::path::PathBuf;
use teleprompt_scene::SceneSettings;

/// What both halves read of a Slidev scene's settings: the files its
/// capture key covers are the files its capture exports.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    /// `deck`: the deck's Markdown, by default `slides.md`.
    pub deck: PathBuf,
}

impl Settings {
    pub(crate) fn of(scene: &impl SceneSettings) -> Self {
        Self {
            deck: scene.path_or("deck", "slides.md"),
        }
    }
}

/// The plugin, to register.
pub fn plugin() -> teleprompt_scene::ScenePlugin {
    teleprompt_scene::ScenePlugin::new(SlidevScene, SlidevRender::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use teleprompt_scene::capture::{session_settings, Session};
    use teleprompt_scene::core::config::SceneConfig;

    /// A scene configured with `settings`, and the session capture makes of it.
    fn halves(settings: serde_json::Value) -> (SceneConfig, Session) {
        let scene = SceneConfig {
            plugin: "slidev".into(),
            settings: serde_json::from_value(settings).unwrap(),
            root: "/project".into(),
        };
        let session = Session {
            scene: "s".into(),
            plugin: "slidev".into(),
            name: None,
            settings: session_settings(&scene.settings),
            root: scene.root.clone(),
            shots: Vec::new(),
        };
        (scene, session)
    }

    #[test]
    fn compile_and_capture_read_the_same_deck() {
        for settings in [
            serde_json::json!({}),
            serde_json::json!({"deck": "talk/deck.md"}),
        ] {
            let (scene, session) = halves(settings);
            assert_eq!(Settings::of(&scene), Settings::of(&session));
        }
        assert_eq!(
            Settings::of(&halves(serde_json::json!({})).0).deck,
            PathBuf::from("/project/slides.md")
        );
    }

    #[test]
    fn the_capture_key_covers_the_deck_capture_exports() {
        use teleprompt_scene::SceneCompiler;
        let (scene, session) = halves(serde_json::json!({"deck": "talk/deck.md"}));
        assert!(SlidevScene
            .inputs(&scene)
            .contains(&Settings::of(&session).deck));
    }
}
