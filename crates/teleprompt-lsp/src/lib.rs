//! teleprompt's language server: a script's problems where they are, as it
//! is typed; completion of attributes, speakers, scenes and policies;
//! hover on a line or block for what it compiles to; go to a speaker's or
//! scene's definition or an included file; and an outline of chapters and
//! lines.
//!
//! The protocol and the text are here; knowing the project is not. What
//! the compiler says about a script comes from an [`Analyzer`], which the
//! CLI implements with the real compile, so this crate depends on core
//! alone and `teleprompt lsp` is the one thing that starts it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use teleprompt_core::Diagnostic;

pub mod complete;
mod server;
pub mod symbols;
pub mod text;

pub use server::{run, serve};

/// Whether `text` is a teleprompt script: its front matter names
/// `teleprompt:`, or it has a ` ```teleprompt ` block. An editor sends the
/// server every Markdown file, and the rest are left alone.
pub fn is_script(text: &str) -> bool {
    let front = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split("\n---").next())
        .is_some_and(|front| front.lines().any(|l| l.starts_with("teleprompt:")));
    front
        || text
            .lines()
            .any(|l| l.trim_start().starts_with("```teleprompt"))
}

/// A named thing a script refers to: a speaker in the cast, or a scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub name: String,
    /// What it is, in a word or two: a speaker's voice, a scene's adapter.
    pub detail: String,
    /// The file it is defined in, and the line there (from 0); `None` for
    /// one built in, such as an adapter used as a scene directly.
    pub location: Option<(PathBuf, u32)>,
}

/// What a script's project offers it, for completion and navigation.
#[derive(Debug, Clone, Default)]
pub struct Project {
    pub speakers: Vec<Definition>,
    pub scenes: Vec<Definition>,
    /// Where the script is, which `include=` paths are relative to.
    pub script_dir: PathBuf,
}

/// What the compiler says about a script's text.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    pub diagnostics: Vec<Diagnostic>,
    /// What a line or block compiles to, as Markdown, by the source line
    /// (from 1) its paragraph or fence starts on.
    pub hovers: BTreeMap<usize, String>,
}

/// The compiler, as the server sees it.
pub trait Analyzer {
    /// The project `script` belongs to.
    fn project(&self, script: &Path) -> Project;
    /// `script`'s problems and what it compiles to, as `text` reads,
    /// saved or not.
    fn analyze(&self, script: &Path, text: &str) -> Analysis;
}
