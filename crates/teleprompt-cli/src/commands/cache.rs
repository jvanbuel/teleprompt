use teleprompt_project::cache::*;

/// `cache`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    /// Shrink the encoded-video cache to this many megabytes, least
    /// recently used first. 0 keeps nothing.
    #[arg(long)]
    pub prune_to_mb: Option<u64>,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    let project = crate::cli::project_here()?;
    let report = run_cache(&project, args.prune_to_mb).map_err(crate::cli::runtime_failure)?;
    crate::cli::emit(format, &report, &report.render());
    Ok(crate::output::Outcome::Ok)
}
