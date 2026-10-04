//! Browser scenes, driven by Playwright.
//!
//! [`scene`] is the compile-time half: it splits a script into shots.
//! [`capture`] runs Playwright and records.

pub mod capture;
pub mod record;
pub mod scene;
pub mod spec;

pub use capture::PlaywrightRender;
pub use record::PlaywrightRecorder;
pub use scene::PlaywrightScene;

/// The adapter, to register.
pub fn adapter() -> teleprompt_plugin::Adapter {
    teleprompt_plugin::Adapter::new(PlaywrightScene, PlaywrightRender::default())
        .recorded_with(PlaywrightRecorder)
}
