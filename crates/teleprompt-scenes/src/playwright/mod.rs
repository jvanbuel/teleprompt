//! Browser scenes, driven by Playwright.
//!
//! [`scene`] is the compile-time half: it splits a script into shots.
//! [`capture`] runs Playwright and records.

pub mod capture;
pub mod record;
pub mod scene;
pub mod spec;
pub mod tools;

pub use capture::PlaywrightRender;
pub use record::PlaywrightRecorder;
pub use scene::PlaywrightScene;

/// The plugin, to register.
pub fn plugin() -> teleprompt_scene::ScenePlugin {
    teleprompt_scene::ScenePlugin::new(PlaywrightScene, PlaywrightRender::default())
        .recorded_with(PlaywrightRecorder)
}
