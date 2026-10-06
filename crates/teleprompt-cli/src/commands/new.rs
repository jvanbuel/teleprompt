use teleprompt::new::*;

/// `new`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    pub path: std::path::PathBuf,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    let report = NewReport {
        created: scaffold(&args.path).map_err(crate::cli::runtime_failure)?,
    };
    crate::cli::emit(format, &report, &report.render());
    Ok(crate::output::Outcome::Ok)
}
