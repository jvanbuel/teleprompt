use teleprompt_setup::*;

/// `setup`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    pub names: Vec<String>,
    /// Run the commands that install what is missing
    #[arg(long)]
    pub run: bool,
    /// Say what teleprompt can be set up to do and what each use still
    /// needs, as the apps ask
    #[arg(long, conflicts_with_all = ["names", "run"])]
    pub uses: bool,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    use crate::cli::{emit, runtime_failure};
    use crate::output::{Format, Outcome};
    let setup = crate::cli::registry().setup_here();
    let reporter = crate::output::Terminal::new(format);
    if args.uses {
        let report = setup.uses();
        emit(format, &report, &report.render());
        return Ok(Outcome::Ok);
    }
    // Asked rather than listed: what to do with teleprompt, not which tools.
    if args.names.is_empty() && !args.run && format == Format::Human && crate::ask::interactive() {
        let (chosen, tools) = crate::ask::choose(&setup).map_err(runtime_failure)?;
        let ran =
            crate::ask::confirm_install(&setup, &tools, &reporter).map_err(runtime_failure)?;
        let mut report = setup.report(&tools, ran);
        report.voice = voice_here()?;
        report.scenes = plugins::scenes(&setup);
        report.voices = plugins::voices(&setup, report.voice.as_ref());
        emit(format, &report, &report.render(&chosen));
        return Ok(Outcome::Ok);
    }
    let tools = resolve(&setup.shipped, &args.names).map_err(runtime_failure)?;
    let ran = if args.run {
        setup.install(&tools, &reporter).map_err(runtime_failure)?
    } else {
        Vec::new()
    };
    let mut report = setup.report(&tools, ran);
    if args.names.is_empty() {
        report.voice = voice_here()?;
    }
    report.scenes = plugins::scenes(&setup);
    report.voices = plugins::voices(&setup, report.voice.as_ref());
    if !args.names.is_empty() {
        report.scenes.retain(|s| args.names.contains(&s.name));
        report.voices.retain(|v| args.names.contains(&v.name));
    }
    emit(format, &report, &report.render(&args.names));
    Ok(Outcome::Ok)
}

/// The voice of the project here, if this is one.
fn voice_here() -> Result<Option<ProjectVoice>, crate::output::Outcome> {
    let Ok(project) = teleprompt_project::project::Project::discover(
        std::path::Path::new("."),
        crate::cli::registry(),
    ) else {
        return Ok(None);
    };
    Ok(Some(
        crate::cli::runtime()?.block_on(teleprompt_project::voice::status(&project)),
    ))
}
