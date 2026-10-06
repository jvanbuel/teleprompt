use std::path::PathBuf;

use teleprompt_project::dub::*;

/// `dub`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub script: crate::cli::ScriptArgs,
    /// Output root; one self-contained directory is written per locale
    #[arg(long)]
    pub out: PathBuf,
    /// Compare against the manifest on disk; exit 3 on drift. Leaves
    /// `--out` untouched, but still synthesizes whatever is not already
    /// cached and writes it to the content-addressed cache — that is
    /// what the comparison measures against
    #[arg(long)]
    pub check: bool,
    /// Also put each shot's clip beside the manifest, as
    /// `clips/<capture_key>.mp4`, recording those not yet captured: one
    /// directory another editor can take whole
    #[arg(long, conflicts_with = "check")]
    pub clips: bool,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    use crate::output::Outcome;
    let script = args.script.open()?;
    let dubber = Dubber::new(&script);
    let runtime = crate::cli::runtime()?;
    if args.check {
        let checked = runtime.block_on(dubber.check(&args.out))?;
        crate::cli::warn(&checked.warnings);
        let drift = &checked.state.drift;
        crate::cli::emit_ok(format, drift, &drift.render(), drift.is_empty());
        return Ok(if drift.is_empty() {
            Outcome::Ok
        } else {
            Outcome::Drift
        });
    }
    let mut dubbed = runtime.block_on(dubber.dub(&args.out))?;
    if args.clips {
        add_clips(&script, &args.out, &mut dubbed)?;
    }
    crate::cli::warn(&dubbed.warnings);
    crate::cli::emit_data(format, &dubbed.manifest, &render_dub(&dubbed));
    Ok(Outcome::Ok)
}
