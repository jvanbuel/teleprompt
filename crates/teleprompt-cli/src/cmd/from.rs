//! `teleprompt from <doc>` — a draft script from a document you already have.
//!
//! The drafting itself is `teleprompt_core::draft`, which is where its tests
//! live. This is the shell around it: read a file, name the chapter that
//! prose before the first heading belongs to, write the result somewhere
//! that is not already occupied.

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_core::draft::{draft, draft_slidev};

/// The stable, typed shape of `from`'s output in both formats.
#[derive(Debug, Serialize)]
pub struct FromReport {
    pub source: PathBuf,
    pub created: PathBuf,
    /// Action blocks written as `review=pending`, which `check` will warn
    /// about until a human has read them.
    pub unreviewed: usize,
    /// Slides of a Slidev deck with no speaker notes, so nothing to say
    /// over them, and so not in the draft.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub silent_slides: Vec<u32>,
    /// What the draft could not follow, one sentence each.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl FromReport {
    pub fn render(&self) -> String {
        let mut s = format!(
            "drafted {} from {}\n",
            self.created.display(),
            self.source.display()
        );
        if self.unreviewed > 0 {
            s.push_str(&format!(
                "  {} tape(s) marked `review=pending` — read them before dubbing\n",
                self.unreviewed
            ));
        }
        if !self.silent_slides.is_empty() {
            let list: Vec<String> = self.silent_slides.iter().map(u32::to_string).collect();
            s.push_str(&format!(
                "  slide(s) {} have no speaker notes, so nothing is said over them and they \
                 are not in the draft\n",
                list.join(", ")
            ));
        }
        for w in &self.warnings {
            s.push_str(&format!("  warning: {w}\n"));
        }
        s
    }
}

/// The chapter name for prose that arrives before the document's own first
/// heading. Derived from the file, because the drafting function would have
/// to invent one and the caller knows a real answer.
fn title_of(doc: &Path) -> String {
    let stem = doc
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Introduction".to_string());
    let spaced = stem.replace(['-', '_'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Introduction".to_string(),
    }
}

/// Where the draft goes when the author does not say: beside the document,
/// named after it, with the extension it will be compiled under.
fn default_out(doc: &Path) -> PathBuf {
    doc.with_extension("teleprompt.md")
}

pub fn run_from(doc: &Path, out: Option<PathBuf>, slidev: bool) -> std::io::Result<FromReport> {
    let source = std::fs::read_to_string(doc)
        .map_err(|e| Error::new(e.kind(), format!("cannot read {}: {e}", doc.display())))?;

    let created = out.unwrap_or_else(|| default_out(doc));
    if created.exists() {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            format!(
                "{} already exists; drafting over it would discard whatever \
                 you have written there",
                created.display()
            ),
        ));
    }

    // A deck is named in the draft's scene config as it was named here:
    // relative to where teleprompt runs, which is where the build will run.
    let (script, silent_slides, warnings) = if slidev {
        // Imports are read relative to the deck's own directory.
        let dir = doc.parent().unwrap_or(Path::new(""));
        let read = |path: &str| std::fs::read_to_string(dir.join(path)).ok();
        let d = draft_slidev(&source, &doc.display().to_string(), &read);
        (d.script, d.silent, d.warnings)
    } else {
        (draft(&source, &title_of(doc)), Vec::new(), Vec::new())
    };
    if let Some(parent) = created.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(&created, &script)?;

    Ok(FromReport {
        source: doc.to_path_buf(),
        created,
        unreviewed: script.matches("review=pending").count(),
        silent_slides,
        warnings,
    })
}
