use std::path::Path;

use teleprompt_compile::CompileOutput;
use teleprompt_core::SpanMs;
use teleprompt_schedule::{diff, Timeline, TimelineDiff, TIMELINE_VERSION};

use crate::project::Project;

/// Compiles the timeline and returns it without writing anything to disk.
pub fn run_plan(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<CompileOutput, Vec<String>> {
    project.compile(script, locale).map(|(out, _)| out)
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

/// Compiles the script and compares it against the committed timeline. With
/// none committed, every item is `added`; one that does not parse is an
/// error.
pub fn run_plan_check(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<TimelineDiff, Vec<String>> {
    let out = run_plan(project, script, locale)?;
    let name = script
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let committed_path = project.timeline_path(&name, locale);

    let committed = if committed_path.exists() {
        let text = std::fs::read_to_string(&committed_path)
            .map_err(|e| vec![format!("cannot read {}: {e}", committed_path.display())])?;
        serde_json::from_str::<Timeline>(&text).map_err(|e| {
            vec![format!(
                "{} is not a valid timeline: {e}",
                committed_path.display()
            )]
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
