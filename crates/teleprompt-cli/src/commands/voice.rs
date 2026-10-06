use teleprompt::voice::clone::*;

/// `voice`'s arguments: what to do with a voice.
#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Make a voice from your takes, so the lines you have not recorded
    /// are spoken in your voice. Clone only your own voice, or one you
    /// have permission to
    Clone {
        /// What to call it: `voice.voice` names it
        name: String,
        /// The language of your takes
        #[arg(long, default_value = "en")]
        language: String,
    },
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    let Command::Clone { name, language } = args.command;
    let project = crate::cli::project_here()?;
    let report = crate::cli::runtime()?
        .block_on(run_clone(&project, &name, &language))
        .map_err(crate::cli::runtime_failure)?;
    crate::cli::emit(format, &report, &report.render());
    Ok(crate::output::Outcome::Ok)
}
