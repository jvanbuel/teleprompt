//! Drafting a script from something you have: a document, a transcript,
//! a recording, or a session recorded while talking (`import`), recorded
//! here first (`record`). What a draft is made of is [`derive`](mod@derive), which is
//! pure; the rest reads files, runs tools and listens.

pub mod derive;
pub mod document;
pub mod import;
pub mod listening;
pub mod record;

use std::path::PathBuf;

use teleprompt_core::Diagnostics;

/// Why a draft was not made. Each says what to do about it, in the words
/// the command prints.
#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    /// The script, or a recording beside it, is there, and `--force` was
    /// not given.
    #[error("{0}")]
    WouldReplace(String),
    /// `record` needs a speech recognizer this build does not have.
    #[error(
        "this teleprompt was built without a speech recognizer: rebuild it \
         with `--features listen`"
    )]
    NoRecognizer,
    /// A recording cannot be transcribed by a build without speech models.
    #[error(
        "this teleprompt was built without speech models, so it cannot transcribe a \
         recording: rebuild it with `--features listen`, or draft from a transcript"
    )]
    CannotTranscribe,
    /// A model named on the command line is not there.
    #[error("no {what} model at {}", .dir.display())]
    NoModel { what: &'static str, dir: PathBuf },
    /// The recorder chosen cannot record here.
    #[error("cannot record with {plugin}: {why}")]
    Recorder { plugin: &'static str, why: String },
    /// A recording that cannot be read back as steps.
    #[error("{}: {source}", path.display())]
    Unreadable {
        path: PathBuf,
        source: teleprompt_scene::record::RecordError,
    },
    /// The session could not be recorded.
    #[error(transparent)]
    Record(#[from] teleprompt_scene::record::RecordError),
    /// What was drafted does not compile: a bug in drafting, not the
    /// author's.
    #[error(
        "the drafted {} does not compile, which is a bug in `import`:\n{}",
        .path.display(),
        .problems.render().join("\n")
    )]
    DraftDoesNotCompile {
        path: PathBuf,
        problems: Diagnostics,
    },
    #[error(transparent)]
    Setup(#[from] teleprompt_setup::SetupError),
    /// Anything else, as it was said: a file that cannot be read, a voice
    /// in which nothing was heard.
    #[error("{0}")]
    Other(String),
}

impl From<String> for DraftError {
    fn from(why: String) -> Self {
        Self::Other(why)
    }
}
