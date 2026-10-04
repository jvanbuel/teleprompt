//! `teleprompt edit <script> <edit>`: one timeline drag, or a line reworded
//! to what its take says, written into the script (`teleprompt_core::edit`).
//! The edited script must still compile, or nothing is written: a drag can
//! move a shot, never break a script.

use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_core::edit::{apply, Edit};

use teleprompt_voice::takes::Takes;

use crate::output::Outcome;
use crate::project::Project;

#[derive(Debug, Serialize)]
pub struct EditReport {
    pub script: PathBuf,
    pub changed: bool,
}

/// Why an edit was not made.
#[derive(Debug)]
pub enum EditError {
    /// The script, or the edit asked of it: each reason on its own.
    Invalid(Vec<String>),
    /// The files could not be read or written.
    Io(String),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(reasons) => write!(f, "{}", reasons.join("\n")),
            Self::Io(message) => write!(f, "{message}"),
        }
    }
}

impl From<EditError> for Outcome {
    fn from(e: EditError) -> Self {
        match e {
            EditError::Invalid(reasons) => Self::ValidationError(reasons),
            EditError::Io(message) => Self::RuntimeFailure(message),
        }
    }
}

fn invalid(reason: String) -> EditError {
    EditError::Invalid(vec![reason])
}

pub fn run_edit(project: &Project, script: &Path, edit: &Edit) -> Result<EditReport, EditError> {
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
    let io = |e: std::io::Error| EditError::Io(format!("cannot write {}: {e}", script.display()));
    std::fs::write(&edited, &after).map_err(io)?;
    let locale = project.source_locale();
    if let Err(errors) = project.compile(&edited, &locale) {
        let _ = std::fs::remove_file(&edited);
        let mut reasons = vec!["not written, since the script would not compile:".to_string()];
        reasons.extend(
            errors
                .iter()
                .map(|e| e.replace(&*edited.to_string_lossy(), &script.to_string_lossy())),
        );
        return Err(EditError::Invalid(reasons));
    }
    let _ = std::fs::remove_file(&edited);
    replace(script, &after).map_err(io)?;
    Ok(report(true))
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

/// Keeps line `line`'s take, or with `None` every take whose line has
/// changed since: the line was corrected, not reworded, so the take still
/// says it. A transcript's misheard words, fixed. `changed` is whether any
/// take was kept.
pub fn run_keep(
    project: &Project,
    script: &Path,
    line: Option<&str>,
) -> Result<EditReport, EditError> {
    let (compiled, _) = project
        .compile(script, &project.source_locale())
        .map_err(EditError::Invalid)?;
    let mut takes = Takes::load(&project.takes_dir()).map_err(|e| EditError::Io(e.to_string()))?;
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
                .map_err(|e| EditError::Io(e.to_string()))?;
            changed = true;
        }
    }
    Ok(EditReport {
        script: script.to_path_buf(),
        changed,
    })
}

/// Rewords line `line` to what its take was heard to say, and keeps the
/// take as the line's: it says what the line now does.
pub fn run_said(project: &Project, script: &Path, line: &str) -> Result<EditReport, EditError> {
    let (compiled, _) = project
        .compile(script, &project.source_locale())
        .map_err(EditError::Invalid)?;
    let text = compiled
        .narration
        .iter()
        .find(|n| n.line_id == line)
        .map(|n| n.text.clone())
        .ok_or_else(|| invalid(format!("no line `{line}` in the script")))?;
    let dir = project.takes_dir();
    let mut takes = Takes::load(&dir).map_err(|e| EditError::Io(e.to_string()))?;
    let said = takes.said(line, &text).ok_or_else(|| {
        invalid(format!(
            "line `{line}` has no take heard saying other words than it does"
        ))
    })?;
    let report = run_edit(
        project,
        script,
        &Edit::Reword {
            line: line.into(),
            text: said.clone(),
        },
    )?;
    takes
        .retext(line, &said)
        .map_err(|e| EditError::Io(e.to_string()))?;
    Ok(report)
}
