use std::path::PathBuf;

use teleprompt_project::build::*;

/// `build`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub script: crate::cli::ScriptArgs,
    /// Where to write the video; defaults to build/<script>.<locale>.mp4 in the project
    #[arg(long)]
    pub out: Option<PathBuf>,
    #[command(flatten)]
    pub frame: crate::cli::FrameArgs,
    /// Re-encode every frame instead of reusing cached ones
    #[arg(long)]
    pub no_cache: bool,
    /// Megabytes of encoded video to keep afterwards; 0 keeps nothing
    #[arg(long)]
    pub cache_max_mb: Option<u64>,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    let script = args.script.open()?;
    let mut builder = Builder::new(&script).frame(args.frame.into());
    if let Some(out) = args.out {
        builder = builder.out(out);
    }
    if args.no_cache {
        builder = builder.no_cache();
    }
    if let Some(mb) = args.cache_max_mb {
        builder = builder.cache_max_mb(mb);
    }
    let reporter = crate::output::Terminal::new(format);
    let report = crate::cli::runtime()?.block_on(builder.build(&reporter))?;
    crate::cli::warn(&report.warnings);
    crate::cli::emit(format, &report, &report.render());
    Ok(crate::output::Outcome::Ok)
}
