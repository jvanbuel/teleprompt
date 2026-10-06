//! `teleprompt edit <script> <edit>`: one timeline drag, or a line reworded
//! to what its take says, written into the script (`teleprompt_script::edit`).
//! The edited script must still compile, or nothing is written: a drag can
//! move a shot, never break a script.

use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_script::edit::apply;
/// A shot's edit, as a timeline drag makes it.
pub use teleprompt_script::edit::Edit;

use teleprompt_voice::takes::Takes;

use crate::project::Script;
use crate::Failure;

#[derive(Debug, Serialize)]
pub struct EditReport {
    pub script: PathBuf,
    pub changed: bool,
}

fn invalid(reason: String) -> Failure {
    Failure::Validation(vec![reason])
}

/// What an edit works on: the script as written, in its source locale,
/// whatever locale it is read in. Takes live beside it in the project.
impl Script {
    /// Makes `edit` to the script, writing it only if it then compiles.
    pub fn edit(&self, edit: &Edit) -> Result<EditReport, Failure> {
        let (project, script) = (self.project(), self.path());
        // A script that cannot be read is invalid, as for every other command.
        let before = std::fs::read_to_string(script)
            .map_err(|e| invalid(format!("cannot read {}: {e}", script.display())))?;
        let after = apply(&before, edit).map_err(invalid)?;
        let report = |changed| EditReport {
            script: script.to_path_buf(),
            changed,
        };
        if after == before {
            return Ok(report(false));
        }
        // Compiled beside the script, where its includes resolve, and written
        // only once it compiles: a script is never left half-edited.
        let mut name = std::ffi::OsString::from(".");
        name.push(script.file_name().unwrap_or_default());
        name.push(".edit");
        let edited = script.with_file_name(name);
        let io =
            |e: std::io::Error| Failure::Runtime(format!("cannot write {}: {e}", script.display()));
        std::fs::write(&edited, &after).map_err(io)?;
        if let Err(errors) = project.script(&edited, project.source_locale()).compile() {
            let _ = std::fs::remove_file(&edited);
            let mut reasons = vec!["not written, since the script would not compile:".to_string()];
            reasons.extend(
                errors
                    .iter()
                    .map(|e| e.replace(&*edited.to_string_lossy(), &script.to_string_lossy())),
            );
            return Err(Failure::Validation(reasons));
        }
        let _ = std::fs::remove_file(&edited);
        replace(script, &after).map_err(io)?;
        Ok(report(true))
    }

    /// Keeps line `line`'s take, or with `None` every take whose line has
    /// changed since: the line was corrected, not reworded, so the take still
    /// says it. A transcript's misheard words, fixed. `changed` is whether any
    /// take was kept.
    pub fn keep_take(&self, line: Option<&str>) -> Result<EditReport, Failure> {
        let (project, script) = (self.project(), self.path());
        let compiled = self
            .in_source_locale()
            .compile()
            .map_err(Failure::Validation)?
            .output;
        let mut takes =
            Takes::load(&project.takes_dir()).map_err(|e| Failure::Runtime(e.to_string()))?;
        if let Some(line) = line {
            if !compiled.narration.iter().any(|n| n.line_id == line) {
                return Err(invalid(format!("no line `{line}` in the script")));
            }
            if takes.iter().all(|(id, _)| id != line) {
                return Err(invalid(format!("line `{line}` has no take to keep")));
            }
        }
        let mut changed = false;
        for n in &compiled.narration {
            let chosen = line.is_none_or(|l| n.line_id == l);
            if chosen && takes.stale(n.line_id.as_str(), &n.text) {
                takes
                    .retext(n.line_id.as_str(), &n.text)
                    .map_err(|e| Failure::Runtime(e.to_string()))?;
                changed = true;
            }
        }
        Ok(EditReport {
            script: script.to_path_buf(),
            changed,
        })
    }

    /// Rewords line `line` to what its take was heard to say, and keeps
    /// the take as the line's: it says what the line now does.
    pub fn keep_said(&self, line: &str) -> Result<EditReport, Failure> {
        let project = self.project();
        let compiled = self
            .in_source_locale()
            .compile()
            .map_err(Failure::Validation)?
            .output;
        let text = compiled
            .narration
            .iter()
            .find(|n| n.line_id == line)
            .map(|n| n.text.clone())
            .ok_or_else(|| invalid(format!("no line `{line}` in the script")))?;
        let dir = project.takes_dir();
        let mut takes = Takes::load(&dir).map_err(|e| Failure::Runtime(e.to_string()))?;
        let said = takes.said(line, &text).ok_or_else(|| {
            invalid(format!(
                "line `{line}` has no take heard saying other words than it does"
            ))
        })?;
        let report = self.edit(&Edit::Reword {
            line: line.into(),
            text: said.clone(),
        })?;
        takes
            .retext(line, &said)
            .map_err(|e| Failure::Runtime(e.to_string()))?;
        Ok(report)
    }
}

/// `path`'s contents, replaced whole or not at all: written beside the
/// file it names, a link followed, with its mode, and renamed onto it.
fn replace(path: &Path, text: &str) -> std::io::Result<()> {
    let real = std::fs::canonicalize(path)?;
    let mut name = std::ffi::OsString::from(".");
    name.push(real.file_name().unwrap_or_default());
    name.push(".partial");
    let partial = real.with_file_name(name);
    let written = std::fs::write(&partial, text)
        .and_then(|()| std::fs::set_permissions(&partial, std::fs::metadata(&real)?.permissions()))
        .and_then(|()| std::fs::rename(&partial, &real));
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    written
}
