//! `teleprompt edit <script> <edit>`: one timeline drag, or a line reworded
//! to what its take says, written into the script (`teleprompt_core::edit`).
//! The edited script must still compile, or nothing is written: a drag can
//! move a shot, never break a script.

use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_core::edit::{apply, Edit};

use teleprompt_voice::takes::Takes;

use crate::cmd::check::compile_script;
use crate::project::Project;

#[derive(Debug, Serialize)]
pub struct EditReport {
    pub script: PathBuf,
    pub changed: bool,
}

pub fn run_edit(project: &Project, script: &Path, edit: &Edit) -> Result<EditReport, String> {
    let before = std::fs::read_to_string(script)
        .map_err(|e| format!("cannot read {}: {e}", script.display()))?;
    let after = apply(&before, edit)?;
    if after == before {
        return Ok(EditReport {
            script: script.to_path_buf(),
            changed: false,
        });
    }
    let write = |text: &str| {
        std::fs::write(script, text).map_err(|e| format!("cannot write {}: {e}", script.display()))
    };
    write(&after)?;
    if let Err(errors) = compile_script(project, script, &crate::cmd::check::source_locale(project))
    {
        write(&before)?;
        return Err(format!(
            "not written, since the script would not compile:\n{}",
            errors.join("\n")
        ));
    }
    Ok(EditReport {
        script: script.to_path_buf(),
        changed: true,
    })
}

/// Rewords line `line` to what its take was heard to say, and keeps the
/// take as the line's: it says what the line now does.
pub fn run_said(project: &Project, script: &Path, line: &str) -> Result<EditReport, String> {
    let (compiled, _) = compile_script(project, script, &crate::cmd::check::source_locale(project))
        .map_err(|e| e.join("\n"))?;
    let text = compiled
        .narration
        .iter()
        .find(|n| n.line_id == line)
        .map(|n| n.text.clone())
        .ok_or_else(|| format!("no line `{line}` in the script"))?;
    let dir = project.takes_dir();
    let mut takes = Takes::load(&dir).map_err(|e| e.to_string())?;
    let said = takes.said(line, &text).ok_or_else(|| {
        format!("line `{line}` has no take heard saying other words than it does")
    })?;
    let report = run_edit(
        project,
        script,
        &Edit::Reword {
            line: line.into(),
            text: said.clone(),
        },
    )?;
    takes.retext(line, &said).map_err(|e| e.to_string())?;
    Ok(report)
}
