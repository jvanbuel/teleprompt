use teleprompt_compile::CompileOutput;
use teleprompt_core::{Diagnostics, SpanMs};
use teleprompt_schedule::{diff, Timeline, TimelineDiff, TIMELINE_VERSION};

use crate::project::Script;

impl Script {
    /// The timeline, compiled without writing anything to disk.
    pub fn plan(&self) -> Result<CompileOutput, Diagnostics> {
        self.compile().map(|c| c.output)
    }

    /// The timeline compared against the committed one. With none
    /// committed, every item is `added`; one that does not parse is an
    /// error.
    pub fn plan_check(&self) -> Result<TimelineDiff, Diagnostics> {
        let out = self.plan()?;
        let name = self
            .path()
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let committed_path = self.project().timeline_path(&name, self.locale());

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
