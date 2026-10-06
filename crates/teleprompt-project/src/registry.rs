//! What this build has to work with: every scene plugin and voice, by name,
//! and what each needs. Built once by the one crate that knows them
//! (`teleprompt-registry`) and handed to each [`Project`](crate::project::Project),
//! so nothing here, and nothing below, knows a plugin or a voice by name.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use teleprompt_plugin::record::Recorder;
use teleprompt_plugin::tool::Tool;
use teleprompt_plugin::{ScenePlugin, ScenePlugins};
use teleprompt_voice::Provider;

use crate::voice::Backends;

/// A voice this build ships, and what it needs that teleprompt does not:
/// a server the author runs, or a key.
pub type Voice = (Provider, &'static Tool);

/// How a `backends:` key naming no voice this build ships is built: it is
/// a server of the author's, under the name they gave it, from its
/// settings; or why those settings do not make one.
pub type OwnServer = fn(
    &str,
    &serde_yaml::Value,
) -> Result<std::sync::Arc<dyn teleprompt_voice::VoiceBackend>, String>;

#[derive(Clone, Copy)]
pub struct Registry {
    /// Every scene plugin: the ones this build ships, then those installed
    /// as programs of their own.
    pub scenes: &'static ScenePlugins,
    /// The names of the scene plugins this build ships, which a program of
    /// the same name does not replace.
    pub shipped_scenes: &'static [&'static str],
    /// The voices this build ships, `null` aside, in the order errors and
    /// `setup` list them.
    pub voices: &'static [Voice],
    /// A voice named in `backends:` that this build does not ship.
    pub own_server: OwnServer,
}

impl Registry {
    /// The mock scene plugin and no voice but `null`: what a test that
    /// compiles a script needs, without a tool behind it.
    pub fn mock() -> Self {
        static SCENES: OnceLock<ScenePlugins> = OnceLock::new();
        Self {
            scenes: SCENES.get_or_init(ScenePlugins::mock),
            shipped_scenes: &["mock"],
            voices: &[],
            own_server: |name, _| Err(format!("no voice backend here makes `{name}`")),
        }
    }

    /// Whether `name` is a scene plugin this build ships.
    pub fn is_shipped_scene(&self, name: &str) -> bool {
        self.shipped_scenes.contains(&name)
    }

    /// What scene plugin `name` runs, capturing and then recording, each
    /// once, by name; `None` when there is no such scene plugin. What
    /// `teleprompt setup <plugin>` installs.
    pub fn scene_needs(&self, name: &str) -> Option<Vec<&'static str>> {
        self.scenes
            .get(name)
            .map(|a| a.needs().iter().map(|t| t.name).collect())
    }

    /// What every scene plugin runs, in the order they are registered.
    pub fn scene_tools(&self) -> Vec<&'static Tool> {
        self.scenes.iter().flat_map(ScenePlugin::needs).collect()
    }

    /// The scene plugins that can record a session, asciinema first: it
    /// records exactly, and what it shows is what was recorded.
    pub fn recorders(&self) -> Vec<Recording> {
        let mut out: Vec<Recording> = self
            .scenes
            .iter()
            .filter_map(|p| {
                p.recorder().map(|recorder| Recording {
                    plugin: p.name(),
                    recorder,
                })
            })
            .collect();
        out.sort_by_key(|r| r.plugin != "asciinema");
        out
    }

    /// What the voices this build ships need that it does not.
    pub fn voice_tools(&self) -> Vec<&'static Tool> {
        self.voices.iter().map(|(_, needs)| *needs).collect()
    }

    /// The voices this build ships, `null` aside, each with what it needs.
    pub fn shipped_voices(&self) -> Vec<(&'static str, &'static Tool)> {
        self.voices
            .iter()
            .map(|(p, needs)| (p.id, *needs))
            .collect()
    }

    /// Whether `name` is a voice this build ships.
    pub fn is_shipped_voice(&self, name: &str) -> bool {
        name == "null" || self.voices.iter().any(|(p, _)| p.id == name)
    }

    /// Every backend this build ships, each built from its own slice of
    /// `settings` (`backends:` in `config_file`).
    pub fn backends(
        &self,
        settings: &BTreeMap<String, serde_yaml::Value>,
        config_file: &str,
    ) -> Backends {
        crate::voice::backends_for(self, settings, config_file)
    }

    /// Settings-free, so every backend gets its defaults: for callers with
    /// no project config in hand, such as tests.
    pub fn default_backends(&self) -> Backends {
        self.backends(&BTreeMap::new(), "teleprompt.toml")
    }

    /// What this build ships, as `setup` lists it.
    pub fn shipped(&self) -> teleprompt_setup::Shipped {
        teleprompt_setup::Shipped {
            scenes: self
                .scenes
                .iter()
                .map(|p| teleprompt_setup::Needs {
                    name: p.name(),
                    tools: p.needs(),
                    built_in: self.is_shipped_scene(p.name()),
                })
                .collect(),
            voices: self.shipped_voices(),
        }
    }

    /// This machine, for what this build needs, with npm packages looked
    /// for in the project around the working directory, if there is one.
    pub fn setup_here(&self) -> teleprompt_setup::Setup {
        let here = std::path::PathBuf::from(".");
        let project = crate::project::Project::discover(&here, *self)
            .map(|p| p.root)
            .unwrap_or(here);
        teleprompt_setup::Setup::detect(self.shipped(), project)
    }
}

/// A scene plugin's recorder, under the plugin's name, which is the scene
/// its drafts run in.
#[derive(Clone, Copy)]
pub struct Recording {
    pub plugin: &'static str,
    recorder: &'static dyn Recorder,
}

impl std::ops::Deref for Recording {
    type Target = dyn Recorder;

    fn deref(&self) -> &Self::Target {
        self.recorder
    }
}
