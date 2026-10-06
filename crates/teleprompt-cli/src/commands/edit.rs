use crate::output::Outcome;

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
    use teleprompt::edit::Edit;
    let project = crate::cli::project_for(&args.script)?;
    let script = project.script(&args.script, project.source_locale());
    let report = match args.edit {
        Command::Said { line } => script.keep_said(&line),
        Command::Keep { line, .. } => {
            script.keep_take(line.as_ref().map(teleprompt_core::LineId::as_str))
        }
        Command::Cue { block, word } => script.edit(&Edit::Cue { block, word }),
        Command::Hold { block } => script.edit(&Edit::Hold { block }),
        Command::Move { block, after, word } => script.edit(&Edit::Move { block, after, word }),
        Command::Stretch { block, by } => script.edit(&Edit::Stretch { block, by }),
        Command::Reword { line, text } => script.edit(&Edit::Reword { line, text }),
        Command::Instruct { line, text } => script.edit(&Edit::Instruct { line, text }),
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
