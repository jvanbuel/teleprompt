//! Browser scenes, driven by Playwright.
//!
//! [`scene`] is the compile-time half: it splits a script into shots.
//! [`capture`] runs Playwright and records.

pub mod capture;
pub mod scene;

pub use capture::PlaywrightRender;
pub use scene::PlaywrightScene;

use std::process::{Command, Stdio};

/// Whether a program can be run.
pub(crate) fn on_path(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}
