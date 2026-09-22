//! Motion graphics, drawn by Remotion.
//!
//! One crate per tool, as with Playwright and vhs: [`scene`] reads a block
//! and says what shots are in it, and [`capture`] hands those shots to
//! Remotion to render. The traits keep the two halves apart where it
//! matters — a `SceneCompiler` has no way to start Node, so `check` and
//! `plan` stay offline whatever lives beside them.
//!
//! This is the other direction from `docs/integrations/remotion.md`. There,
//! Remotion owns the whole video and reads teleprompt's manifest. Here,
//! teleprompt owns the video and Remotion draws one scene of it — a title,
//! a diagram, a chart — at exactly the length the narration over it runs.

pub mod capture;
pub mod scene;

pub use capture::RemotionRender;
pub use scene::RemotionScene;

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
