//! A scene's settings read the same from both halves of a plugin: from the
//! project's scene config at compile, from the session at capture.

use std::path::PathBuf;

use teleprompt_scene::capture::{session_settings, Session};
use teleprompt_scene::core::config::SceneConfig;
use teleprompt_scene::SceneSettings;

fn halves(settings: serde_json::Value) -> (SceneConfig, Session) {
    let scene = SceneConfig {
        plugin: "any".into(),
        settings: serde_json::from_value(settings).unwrap(),
        root: "/project".into(),
    };
    let session = Session {
        scene: "s".into(),
        plugin: "any".into(),
        name: None,
        settings: session_settings(&scene.settings),
        root: scene.root.clone(),
        shots: Vec::new(),
    };
    (scene, session)
}

#[test]
fn a_path_setting_reads_the_same_given_or_not() {
    let (scene, session) = halves(serde_json::json!({"deck": "talk/deck.md"}));
    assert_eq!(
        scene.path_or("deck", "slides.md"),
        PathBuf::from("/project/talk/deck.md")
    );
    assert_eq!(
        scene.path_or("deck", "slides.md"),
        session.path_or("deck", "slides.md")
    );
    assert_eq!(
        scene.path_or("dir", "media"),
        PathBuf::from("/project/media")
    );
    assert_eq!(
        scene.path_or("dir", "media"),
        session.path_or("dir", "media")
    );
}

#[test]
fn a_session_carries_scalars_as_text_and_one_level_of_nesting() {
    let (_, session) = halves(serde_json::json!({
        "font_size": 32, "dark": true, "env": {"PATH": "/bin"}, "list": [1]
    }));
    assert_eq!(session.value("font_size"), Some("32"));
    assert_eq!(session.value("dark"), Some("true"));
    assert_eq!(session.value("env.PATH"), Some("/bin"));
    assert_eq!(session.value("list"), None);
}
