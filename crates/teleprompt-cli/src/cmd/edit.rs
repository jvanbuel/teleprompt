//! `teleprompt edit <script> <edit>`: one timeline drag, or a line reworded
//! to what its take says, written into the script (`teleprompt_core::edit`).
//! The edited script must still compile, or nothing is written: a drag can
//! move a shot, never break a script.

use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_core::edit::{apply, Edit};

use teleprompt_voice::takes::Takes;

use crate::output::{Failure, Outcome};
use crate::project::Project;

#[derive(Debug, Serialize)]
pub struct EditReport {
    pub script: PathBuf,
    pub changed: bool,
}

fn invalid(reason: String) -> Failure {
    Failure::Validation(vec![reason])
}

pub fn run_edit(project: &Project, script: &Path, edit: &Edit) -> Result<EditReport, Failure> {
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
    let locale = project.source_locale();
    if let Err(errors) = project.compile(&edited, &locale) {
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
) -> Result<EditReport, Failure> {
    let (compiled, _) = project
        .compile(script, &project.source_locale())
        .map_err(Failure::Validation)?;
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

/// Rewords line `line` to what its take was heard to say, and keeps the
/// take as the line's: it says what the line now does.
pub fn run_said(project: &Project, script: &Path, line: &str) -> Result<EditReport, Failure> {
    let (compiled, _) = project
        .compile(script, &project.source_locale())
        .map_err(Failure::Validation)?;
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
        .map_err(|e| Failure::Runtime(e.to_string()))?;
    Ok(report)
}

/// `edit`'s arguments: the script, and the edit.
#[derive(clap::Args)]
pub struct Args {
    pub script: std::path::PathBuf,
    #[command(subcommand)]
    pub edit: Command,
}

/// One edit `edit` makes, naming blocks and lines by their ids in `plan`.
#[derive(clap::Subcommand)]
pub enum Command {
    /// Run the block with its line, from the line's WORDth word (0: with
    /// the line)
    Cue {
        block: teleprompt_core::BlockId,
        #[arg(long)]
        word: usize,
    },
    /// Run the block after its line
    Hold { block: teleprompt_core::BlockId },
    /// Put the block after another line: held, or cued at --word
    Move {
        block: teleprompt_core::BlockId,
        #[arg(long)]
        after: teleprompt_core::LineId,
        #[arg(long)]
        word: Option<usize>,
    },
    /// Make the block's shots BY times longer (above 1) or shorter
    Stretch {
        block: teleprompt_core::BlockId,
        #[arg(long)]
        by: f64,
    },
    /// Reword the line to what its take was heard to say, keeping the take
    Said { line: teleprompt_core::LineId },
    /// Keep the line's take though its words changed: they were corrected,
    /// not reworded, as a transcript's misheard words are
    Keep {
        line: Option<teleprompt_core::LineId>,
        /// Every take whose line changed since it was recorded
        #[arg(long, conflicts_with = "line", required_unless_present = "line")]
        all: bool,
    },
    /// Say the line as TEXT, keeping its id and attributes. A take of the
    /// old words becomes one to record again
    Reword {
        line: teleprompt_core::LineId,
        text: String,
    },
    /// Tell the voice how to say the line ("slower, amused"), for a
    /// backend that takes instructions; without TEXT, remove it
    Instruct {
        line: teleprompt_core::LineId,
        text: Option<String>,
    },
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    use teleprompt_core::edit::Edit;
    let script = &args.script;
    let project = crate::cli::project_for(script)?;
    let report = match args.edit {
        Command::Said { line } => run_said(&project, script, &line),
        Command::Keep { line, .. } => run_keep(
            &project,
            script,
            line.as_ref().map(teleprompt_core::LineId::as_str),
        ),
        Command::Cue { block, word } => run_edit(&project, script, &Edit::Cue { block, word }),
        Command::Hold { block } => run_edit(&project, script, &Edit::Hold { block }),
        Command::Move { block, after, word } => {
            run_edit(&project, script, &Edit::Move { block, after, word })
        }
        Command::Stretch { block, by } => run_edit(&project, script, &Edit::Stretch { block, by }),
        Command::Reword { line, text } => run_edit(&project, script, &Edit::Reword { line, text }),
        Command::Instruct { line, text } => {
            run_edit(&project, script, &Edit::Instruct { line, text })
        }
    }
    .map_err(Outcome::from)?;
    let human = if report.changed {
        format!("edited {}\n", report.script.display())
    } else {
        "nothing to change\n".to_string()
    };
    crate::cli::emit(format, &report, &human);
    Ok(Outcome::Ok)
}
