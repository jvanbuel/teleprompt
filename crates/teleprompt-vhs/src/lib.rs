//! Terminal scenes, recorded by VHS.
//!
//! [`scene`] is the compile-time half: it reads a tape, splits it into
//! shots and says how long each takes. [`capture`] runs `vhs` and records.

pub mod capture;
pub mod scene;

pub use capture::VhsRender;
pub use scene::VhsScene;

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
