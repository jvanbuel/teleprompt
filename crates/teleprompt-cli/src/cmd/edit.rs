//! `teleprompt edit <script> <edit>`: one timeline drag, written into the
//! script (`teleprompt_core::edit`). The edited script must still compile,
//! or nothing is written: a drag can move a shot, never break a script.

use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_core::edit::{apply, Edit};

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
    if let Err(errors) = compile_script(project, script, "en") {
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
