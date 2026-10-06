use teleprompt_project::plan::*;

/// `plan`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub script: crate::cli::ScriptArgs,
    /// Compare against the committed timeline instead: print what
    /// changed, and exit 3 if anything did
    #[arg(long)]
    pub check: bool,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    use crate::output::Outcome;
    let script = args.script.open()?;
    if args.check {
        let d = script.plan_check().map_err(Outcome::ValidationError)?;
        crate::cli::emit_ok(format, &d, &format!("{}\n", d.render()), d.is_empty());
        return Ok(if d.is_empty() {
            Outcome::Ok
        } else {
            Outcome::Drift
        });
    }
    let out = script.plan().map_err(Outcome::ValidationError)?;
    crate::cli::emit_data(format, &out.timeline, &render_plan(&out));
    let narrated = out
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref());
    let total = narrated.clone().count();
    let estimated = narrated
        .filter(|n| n.duration_source == teleprompt_core::DurationSource::Estimated)
        .count();
    if estimated > 0 {
        eprintln!(
            "warning: {estimated} of {total} narration durations are \
             estimated; run `teleprompt dub` to measure them before \
             committing this timeline"
        );
    }
    Ok(Outcome::Ok)
}
