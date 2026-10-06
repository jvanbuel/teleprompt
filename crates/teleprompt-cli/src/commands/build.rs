use std::path::PathBuf;

use teleprompt::build::*;

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
    let report = crate::cli::runtime()?.block_on(builder.build(&mut progress_reporter(format)))?;
    crate::cli::warn(&report.warnings);
    crate::cli::emit(format, &report, &report.render());
    Ok(crate::output::Outcome::Ok)
}

/// A render's progress, each percent of it: to a human at a terminal as a
/// line that rewrites itself with a carriage return, which a log cannot
/// take; with `--format json`, as progress events.
fn progress_reporter(format: crate::output::Format) -> impl FnMut(teleprompt_render::Progress) {
    use crate::output::Format;
    use std::io::{IsTerminal, Write};

    let show = format == Format::Human && std::io::stderr().is_terminal();
    let mut last = u64::MAX;
    move |p: teleprompt_render::Progress| {
        if p.of_ms == 0 {
            return;
        }
        let percent = (p.rendered_ms.min(p.of_ms) * 100) / p.of_ms;
        if percent == last {
            return;
        }
        last = percent;
        if format == Format::Json {
            teleprompt::progress::progress(
                "render",
                String::new,
                serde_json::json!({ "done_ms": p.rendered_ms.min(p.of_ms), "of_ms": p.of_ms }),
            );
            return;
        }
        if !show {
            return;
        }
        let mut err = std::io::stderr();
        let _ = write!(err, "\r  rendering  {percent:>3}%");
        if percent == 100 {
            let _ = writeln!(err);
        }
        let _ = err.flush();
    }
}
