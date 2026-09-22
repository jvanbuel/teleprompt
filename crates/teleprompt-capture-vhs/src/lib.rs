//! Recording a terminal scene by handing `vhs` a re-timed tape.
//!
//! teleprompt already re-writes every tape so a cue lasts exactly as long
//! as the sentence over it, so the tape given to `vhs` *is* the schedule:
//! run it, and the video's timeline is the timeline. One run per session —
//! the cues are concatenated in order so the program stays running across
//! cues — and the cues are windows onto the one video, at the offsets
//! the tape was written to produce.
//!
//! Nothing here interprets the tape. `vhs` was built to run tapes; the
//! only thing teleprompt has to know is how to write one.

pub mod render;

pub use render::VhsRender;

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
