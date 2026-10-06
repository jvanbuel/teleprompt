//! The scene plugins installed as programs of their own, each asked what
//! it is (docs/guide/scene-plugins.md).

use serde::Serialize;
use tabled::builder::Builder;
use tabled::settings::object::Columns;
use tabled::settings::{Modify, Style, Width};
use teleprompt_plugin::protocol::host::{self, Plugin};

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
                Some("the built-in plugin of this name is used instead".to_string())
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

/// `plugins` as a table, saying where more go when given `dir`; nothing
/// when there are none.
pub fn render(plugins: &[Installed], dir: Option<&str>) -> String {
    if plugins.is_empty() {
        return String::new();
    }
    let mut table = Builder::default();
    table.push_record(["Plugin", "Status", "Needs", "Path"]);
    for p in plugins {
        let status = match &p.problem {
            Some(why) => format!("not used: {why}"),
            None => "ok".to_string(),
        };
        let needs = if p.needs.is_empty() {
            "-".to_string()
        } else {
            p.needs.join(", ")
        };
        table.push_record([p.name.clone(), status, needs, p.path.clone()]);
    }
    let table = table
        .build()
        .with(Style::sharp())
        .with(Modify::new(Columns::one(1)).with(Width::wrap(40).keep_words(true)))
        .to_string();
    let more = dir.map_or(String::new(), |dir| {
        format!("Plugins are found on PATH or in {dir}.\n")
    });
    format!("\nScene plugins installed as programs:\n{table}\n{more}")
}
