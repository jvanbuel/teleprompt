//! The scene plugins and voices this teleprompt can use, as `setup` lists
//! them: scene plugins built in or installed as programs of their own
//! (docs/guide/scene-plugins.md), and the voices it ships
//! (docs/guide/voices.md).

use serde::Serialize;
use tabled::builder::Builder;
use tabled::settings::object::Columns;
use tabled::settings::{Modify, Style, Width};
use teleprompt_plugin::protocol::host::{self, Plugin};
use teleprompt_plugin::tool::Tool;

use super::{ProjectVoice, Setup};

#[derive(Debug, Serialize)]
pub struct Scene {
    pub name: String,
    /// The program, for a plugin installed as one; `null` when built in.
    pub path: Option<String>,
    /// What it needs, by name.
    pub needs: Vec<String>,
    /// Which of those are not on this machine.
    pub missing: Vec<String>,
    /// Why it cannot be used: it does not answer as a plugin, or a built-in
    /// one has its name.
    pub problem: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Voice {
    pub name: String,
    /// Where to read about its server or service.
    pub home: String,
    /// Whether the project here chose it.
    pub chosen: bool,
}

/// Every scene plugin: the built-in ones, then each installed as a program,
/// asked what it needs.
pub fn scenes(setup: &Setup) -> Vec<Scene> {
    let row = |name: &str, path: Option<String>, needs: &[&Tool], problem: Option<String>| Scene {
        name: name.to_string(),
        path,
        needs: needs.iter().map(|t| t.name.to_string()).collect(),
        missing: needs
            .iter()
            .filter(|t| setup.missing(t))
            .map(|t| t.name.to_string())
            .collect(),
        problem,
    };
    let registry = setup.registry;
    let built_in = registry
        .scenes
        .iter()
        // `mock` stands in for the others in tests.
        .filter(|p| registry.is_shipped_scene(p.name()) && p.name() != "mock")
        .map(|p| row(p.name(), None, &p.needs(), None));
    let programs = host::discover().into_iter().map(|found| {
        let name = found.name.clone();
        let path = Some(found.path.display().to_string());
        if registry.is_shipped_scene(&name) {
            let why = "the built-in plugin of this name is used instead".to_string();
            return row(&name, path, &[], Some(why));
        }
        let plugin = Plugin::new(found);
        match plugin.describe() {
            Ok(_) => row(&name, path, plugin.needs(), None),
            Err(why) => row(&name, path, &[], Some(why)),
        }
    });
    built_in.chain(programs).collect()
}

/// Every voice this build ships, and the project's own when it is a server
/// of the author's.
pub fn voices(setup: &Setup, chosen: Option<&ProjectVoice>) -> Vec<Voice> {
    let chosen = chosen.map(|v| v.backend.as_str());
    let mut out: Vec<Voice> = setup
        .registry
        .shipped_voices()
        .into_iter()
        .map(|(name, needs)| Voice {
            name: name.to_string(),
            home: needs.home.to_string(),
            chosen: chosen == Some(name),
        })
        .collect();
    if let Some(name) = chosen.filter(|n| !out.iter().any(|v| v.name == *n)) {
        let home = if name == "null" {
            "silent"
        } else {
            "your own server"
        };
        out.push(Voice {
            name: name.to_string(),
            home: home.to_string(),
            chosen: true,
        });
    }
    out
}

/// The scene plugins and the voices as a table each, with where plugins
/// are found when given `dir`; a table with no rows is left out.
pub fn render(scenes: &[Scene], voices: &[Voice], dir: Option<&str>) -> String {
    let mut out = String::new();
    if !scenes.is_empty() {
        let rows = scenes.iter().map(|s| {
            let status = match (&s.problem, s.missing.is_empty()) {
                (Some(why), _) => format!("not used: {why}"),
                (None, true) => "ready".to_string(),
                (None, false) => format!("missing {}", s.missing.join(", ")),
            };
            let needs = if s.needs.is_empty() {
                "-".to_string()
            } else {
                s.needs.join(", ")
            };
            let from = s.path.clone().unwrap_or_else(|| "built in".to_string());
            [s.name.clone(), status, needs, from]
        });
        out.push_str(&format!(
            "\nScene plugins\n{}\n",
            table(["Plugin", "Status", "Needs", "From"], rows)
        ));
        if let Some(dir) = dir {
            out.push_str(&format!("Plugins are found on PATH or in {dir}.\n"));
        }
    }
    if !voices.is_empty() {
        let rows = voices.iter().map(|v| {
            let status = if v.chosen { "chosen here" } else { "-" };
            [v.name.clone(), status.to_string(), v.home.clone()]
        });
        out.push_str(&format!(
            "\nVoices\n{}\n",
            table(["Voice", "Project", "About"], rows)
        ));
    }
    out
}

/// `rows` under `head`, the second column (a status) wrapped to stay readable.
fn table<const N: usize>(head: [&str; N], rows: impl Iterator<Item = [String; N]>) -> String {
    let mut builder = Builder::default();
    builder.push_record(head);
    for row in rows {
        builder.push_record(row);
    }
    builder
        .build()
        .with(Style::sharp())
        .with(Modify::new(Columns::one(1)).with(Width::wrap(48).keep_words(true)))
        .to_string()
}
