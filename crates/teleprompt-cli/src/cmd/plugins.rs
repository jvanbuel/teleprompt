//! `teleprompt plugins`: the scene plugins installed as programs of their
//! own, each asked what it is (docs/guide/scene-plugins.md).

use serde::Serialize;
use teleprompt_plugin::protocol::host::{self, Plugin};

use crate::cli::Run;
use crate::output::{Format, Outcome};

/// `plugins` takes no arguments of its own.
#[derive(clap::Args)]
pub struct Args {}

#[derive(Debug, Serialize)]
pub struct PluginsReport {
    pub plugins: Vec<Installed>,
    /// Where plugins go when not on PATH.
    pub dir: String,
}

#[derive(Debug, Serialize)]
pub struct Installed {
    pub name: String,
    pub path: String,
    /// Why it cannot be used: it does not answer as a plugin, or a built-in
    /// one has its name.
    pub problem: Option<String>,
    /// What it needs, by name, as it describes it.
    pub needs: Vec<String>,
}

/// Every installed plugin, each asked to describe itself.
pub fn installed() -> Vec<Installed> {
    host::discover()
        .into_iter()
        .map(|found| {
            let shadowed = crate::scene::is_built_in(&found.name);
            let name = found.name.clone();
            let path = found.path.display().to_string();
            let plugin = Plugin::new(found);
            let problem = if shadowed {
                Some(format!(
                    "teleprompt has a built-in scene plugin named `{name}`, which is used instead"
                ))
            } else {
                plugin.describe().err()
            };
            let needs = if problem.is_some() {
                Vec::new()
            } else {
                plugin.needs().iter().map(|t| t.name.to_string()).collect()
            };
            Installed {
                name,
                path,
                problem,
                needs,
            }
        })
        .collect()
}

pub fn run(_: Args, format: Format) -> Run {
    let report = PluginsReport {
        plugins: installed(),
        dir: host::plugins_dir().display().to_string(),
    };
    crate::cli::emit(format, &report, &render(&report));
    Ok(Outcome::Ok)
}

fn render(report: &PluginsReport) -> String {
    if report.plugins.is_empty() {
        return format!(
            "No plugins installed. A scene plugin is a program named \
             teleprompt-scene-<name>, on PATH or in {}: docs/guide/scene-plugins.md\n",
            report.dir
        );
    }
    let mut out = String::new();
    for p in &report.plugins {
        out.push_str(&format!("{:<18} {}\n", p.name, p.path));
        match &p.problem {
            Some(why) => out.push_str(&format!("{:<18} unusable: {why}\n", "")),
            None if !p.needs.is_empty() => {
                out.push_str(&format!("{:<18} needs {}\n", "", p.needs.join(", ")));
            }
            None => {}
        }
    }
    out
}
