//! `teleprompt from <doc>` — a draft script from a document you already have.
//!
//! The drafting itself is `teleprompt_core::draft`, which is where its tests
//! live. This is the shell around it: read a file, name the chapter that
//! prose before the first heading belongs to, write the result somewhere
//! that is not already occupied.

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_core::draft::draft;

/// The stable, typed shape of `from`'s output in both formats.
#[derive(Debug, Serialize)]
pub struct FromReport {
    pub source: PathBuf,
    pub created: PathBuf,
    /// Action blocks written as `review=pending`, which `check` will warn
    /// about until a human has read them.
    pub unreviewed: usize,
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

pub fn run_from(doc: &Path, out: Option<PathBuf>) -> std::io::Result<FromReport> {
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

    let script = draft(&source, &title_of(doc));
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
    })
}
