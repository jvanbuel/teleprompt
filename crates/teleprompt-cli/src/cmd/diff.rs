use std::path::Path;

use teleprompt_schedule::{diff, Timeline, TimelineDiff, TIMELINE_VERSION};

use crate::cmd::plan::run_plan;
use crate::project::Project;

/// Compiles the script fresh and compares it against the committed
/// timeline. A missing committed timeline is not an error — it is a first
/// run, and every item in the freshly compiled timeline is reported as
/// `added` so the author sees what they are about to commit. A committed
/// timeline that exists but fails to parse is a real error.
pub fn run_diff(
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
            duration_ms: 0,
            generated_by: String::new(),
            entries: Vec::new(),
        }
    };

    Ok(diff(&committed, &out.timeline))
}
