//! `teleprompt cache`: what the project's caches hold, and shrinking them.

use std::path::PathBuf;

use serde::Serialize;
use teleprompt_project::cache::{Pruned, Stats};
use teleprompt_project::project::Project;

#[derive(Debug, Serialize)]
pub struct CacheReport {
    pub root: PathBuf,
    pub voice: Stats,
    pub compose: Stats,
    /// What a `prune` did, or `null` for a report that only looked.
    pub pruned: Option<Pruned>,
}

impl CacheReport {
    pub fn render(&self) -> String {
        let mut out = format!("  {}\n", self.root.display());
        out.push_str(&format!(
            "  voice            {} entries, {}\n",
            self.voice.entries,
            size(self.voice.bytes)
        ));
        out.push_str(&format!(
            "  compose          {} entries, {}\n",
            self.compose.entries,
            size(self.compose.bytes)
        ));
        if let Some(pruned) = self.pruned {
            out.push_str(&format!(
                "  pruned           {} entries, {} freed\n",
                pruned.removed,
                size(pruned.freed)
            ));
        }
        out
    }
}

/// Bytes, in the unit a human would have said them in.
fn size(bytes: u64) -> String {
    const MB: f64 = 1_048_576.0;
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.0} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / MB),
    }
}

/// Report the caches, having first pruned the compose cache to `max_mb` if
/// one was asked for.
pub fn run_cache(project: &Project, max_mb: Option<u64>) -> std::io::Result<CacheReport> {
    let pruned = match max_mb {
        Some(mb) => Some(teleprompt_project::cache::prune(
            &project.caches().compose(),
            mb * 1_048_576,
        )?),
        None => None,
    };
    Ok(CacheReport {
        root: project.caches().root,
        voice: teleprompt_project::cache::stats(&project.caches().voice()),
        compose: teleprompt_project::cache::stats(&project.caches().compose()),
        pruned,
    })
}

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
