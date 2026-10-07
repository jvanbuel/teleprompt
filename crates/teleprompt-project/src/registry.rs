//! What this build has to work with: every scene plugin and voice, by name,
//! and what each needs. Built once by the one module that knows them
//! (the command line's `registry`) and handed to each [`Project`](crate::project::Project),
//! so nothing here, and nothing below, knows a plugin or a voice by name.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use teleprompt_core::tool::Tool;
use teleprompt_scene::record::Recorder;
use teleprompt_scene::{ScenePlugin, ScenePlugins};
use teleprompt_voice::backends::Backends;
use teleprompt_voice::catalogue::VoiceCatalogue;

#[derive(Clone, Copy)]
pub struct Registry {
    /// Every scene plugin: the ones this build ships, then those installed
    /// as programs of their own.
    pub scenes: &'static ScenePlugins,
    /// The names of the scene plugins this build ships, which a program of
    /// the same name does not replace.
    pub shipped_scenes: &'static [&'static str],
    /// Every voice this build has.
    pub voices: &'static VoiceCatalogue,
}

impl Registry {
    /// The mock scene plugin and no voice but `null`: what a test that
    /// compiles a script needs, without a tool behind it.
    pub fn mock() -> Self {
        static SCENES: OnceLock<ScenePlugins> = OnceLock::new();
        Self {
            scenes: SCENES.get_or_init(ScenePlugins::mock),
            shipped_scenes: &["mock"],
            voices: &teleprompt_voice::catalogue::NONE,
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

    /// Every backend this build ships, each built from its own slice of
    /// `settings` (`backends:` in `config_file`).
    pub fn backends(
        &self,
        settings: &BTreeMap<String, serde_yaml::Value>,
        config_file: &str,
    ) -> Backends {
        Backends::new(self.voices, settings, config_file)
    }

    /// Settings-free, so every backend gets its defaults: for callers with
    /// no project config in hand, such as tests.
    pub fn default_backends(&self) -> Backends {
        self.backends(&BTreeMap::new(), "teleprompt.toml")
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
