//! How a long step tells whoever runs it how far it got, and asks what
//! only they can answer. A library reports through a [`Reporter`] and
//! prints nothing itself; the front end decides what a person sees, or
//! what an app reads.

use serde::Serialize;

use crate::{LineId, ShotId};

/// One step of a long command. As JSON it is the event an app reads,
/// `{"stage": …, …}`, under `"event": "progress"`
/// (docs/design.md#cli).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "stage", rename_all = "lowercase")]
pub enum Progress {
    /// A line's audio made: `done` of `of` lines, in completion order.
    Voice {
        done: usize,
        of: usize,
        line: LineId,
    },
    /// A shot recorded: `done` of `of`.
    Capture {
        done: usize,
        of: usize,
        scene: String,
        shot: ShotId,
    },
    /// The video rendered this far.
    Render { done_ms: u64, of_ms: u64 },
    /// A tool being installed.
    Install {
        tool: String,
        #[serde(flatten)]
        state: Install,
    },
}

/// Where a tool's install has got to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Install {
    /// Its command is about to run.
    Start {
        command: String,
    },
    /// Its download has `mb` of `of` megabytes on disk.
    Downloading {
        mb: u64,
        of: u64,
    },
    Done,
}

/// Whoever a long step reports to: the command line, the prompter's page,
/// or nobody.
pub trait Reporter: Sync {
    fn progress(&self, progress: Progress);

    /// A line for a person, when what a step does is not a count: what it
    /// is recording with, what it is transcribing. Nothing, for an app.
    fn note(&self, _text: &str) {}

    /// Whether `names`, needed to `why`, may be installed now: asks the
    /// person, installs, and says whether they are there. Nobody to ask,
    /// nothing installed: `false`.
    fn offer(&self, _names: &[&str], _why: &str) -> bool {
        false
    }

    /// Whether a person is watching: a tool's own output may be shown to
    /// them, and it may ask them things.
    fn attended(&self) -> bool {
        false
    }
}

/// Reports to nobody: for a test, or a step run for an app that wants only
/// its result.
pub struct Silent;

impl Reporter for Silent {
    fn progress(&self, _: Progress) {}
}

impl<F: Fn(Progress) + Sync> Reporter for F {
    fn progress(&self, progress: Progress) {
        self(progress);
    }
}
