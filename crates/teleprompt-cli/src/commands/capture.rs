/// `capture`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub script: crate::cli::ScriptArgs,
    #[command(flatten)]
    pub frame: crate::cli::FrameArgs,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    let script = args.script.open()?;
    let builder = teleprompt::build::Builder::new(&script).frame(args.frame.into());
    let report = crate::cli::runtime()?
        .block_on(builder.capture(&mut teleprompt::progress::capture_progress))?;
    crate::cli::warn(&report.warnings);
    crate::cli::emit(format, &report, &report.render());
    Ok(crate::output::Outcome::Ok)
}
