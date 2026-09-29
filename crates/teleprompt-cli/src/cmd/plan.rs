use std::path::Path;

use teleprompt_compile::CompileOutput;

use crate::cmd::check::compile_script;
use crate::project::Project;

/// Compiles the timeline and returns it without writing anything to disk.
pub fn run_plan(
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<CompileOutput, Vec<String>> {
    compile_script(project, script, locale).map(|(out, _)| out)
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
