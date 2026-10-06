use teleprompt::translate::*;

/// `translate`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    pub script: std::path::PathBuf,
    /// The locale to translate into, e.g. nl, fr, pt-BR
    #[arg(long, value_parser = crate::cli::language_tag)]
    pub to: String,
    /// Translate with this provider: ollama, openai, claude or command
    #[arg(long, conflicts_with = "command")]
    pub provider: Option<String>,
    /// The provider's model
    #[arg(long, conflicts_with = "command")]
    pub model: Option<String>,
    /// Translate with this shell command: it gets the request as JSON on
    /// stdin and answers {"items": [{"id", "text"}]} on stdout
    #[arg(long)]
    pub command: Option<String>,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    use crate::output::Outcome;
    let choice = Choice {
        provider: args.provider.as_deref(),
        model: args.model.as_deref(),
        command: args.command.as_deref(),
    };
    let project = crate::cli::project_for(&args.script)?;
    let script = project.script(&args.script, &args.to);
    let translator = script.translator(&choice)?;
    let report = crate::cli::runtime()?.block_on(script.translate(&translator))?;
    crate::cli::emit(format, &report, &report.render());
    Ok(Outcome::Ok)
}
