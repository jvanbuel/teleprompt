//! `teleprompt plan`: the timeline, and how it differs from the committed one.

use teleprompt_core::{Diagnostics, SpanMs};
use teleprompt_pipeline::schedule::{diff, Timeline, TimelineDiff, TIMELINE_VERSION};
use teleprompt_project::CompileOutput;

use teleprompt_project::project::Script;

/// The timeline, compiled without writing anything to disk.
pub fn plan(script: &Script) -> Result<CompileOutput, Diagnostics> {
    script.compile().map(|c| c.output)
}

/// The timeline compared against the committed one. With none
/// committed, every item is `added`; one that does not parse is an
/// error.
pub fn plan_check(script: &Script) -> Result<TimelineDiff, Diagnostics> {
    let out = plan(script)?;
    let name = script
        .path()
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let committed_path = script.project().timeline_path(&name, script.locale());

    let committed = if committed_path.exists() {
        let text = std::fs::read_to_string(&committed_path).map_err(|e| {
            Diagnostics::error(format!("cannot read {}: {e}", committed_path.display()))
        })?;
        serde_json::from_str::<Timeline>(&text).map_err(|e| {
            Diagnostics::error(format!(
                "{} is not a valid timeline: {e}",
                committed_path.display()
            ))
        })?
    } else {
        Timeline {
            version: TIMELINE_VERSION,
            script: out.timeline.script.clone(),
            locale: out.timeline.locale.clone(),
            duration_ms: SpanMs::ZERO,
            generated_by: String::new(),
            entries: Vec::new(),
        }
    };

    Ok(diff(&committed, &out.timeline))
}

/// Renders a `plan` result as a short prose report, one line per item.
pub fn render_plan(out: &CompileOutput) -> String {
    let mut s = format!(
        "{} ({}) — {:.1}s across {} item(s)\n",
        out.timeline.script,
        out.timeline.locale,
        out.timeline.duration_ms.ms() as f64 / 1000.0,
        out.timeline.entries.len()
    );
    for e in &out.timeline.entries {
        let narration = e
            .narration
            .as_ref()
            .map(|n| format!("{:.1}s {}", n.duration_ms.ms() as f64 / 1000.0, n.line))
            .unwrap_or_else(|| "—".to_string());
        s.push_str(&format!(
            "  {:>8.1}s  {:<10} {}\n",
            e.start_ms.ms() as f64 / 1000.0,
            e.policy,
            narration
        ));
    }
    for w in &out.warnings {
        s.push_str(&format!("  warning: {w}\n"));
    }
    s
}

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
        let d = plan_check(&script).map_err(|p| Outcome::ValidationError(p.render()))?;
        crate::cli::emit_ok(format, &d, &format!("{}\n", d.render()), d.is_empty());
        return Ok(if d.is_empty() {
            Outcome::Ok
        } else {
            Outcome::Drift
        });
    }
    let out = plan(&script).map_err(|p| Outcome::ValidationError(p.render()))?;
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
