//! What this build has to work with: every scene plugin and voice, by name,
//! and what each needs. Built once by the one module that knows them
//! (the command line's `registry`) and handed to each [`Project`](crate::project::Project),
//! so nothing here, and nothing below, knows a plugin or a voice by name.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use teleprompt_scene::ScenePlugins;
use teleprompt_voice::backends::Backends;
use teleprompt_voice::catalogue::VoiceCatalogue;

#[derive(Clone, Copy)]
pub struct Registry {
    /// Every scene plugin: the ones this build ships, then those installed
    /// as programs of their own.
    pub scenes: &'static ScenePlugins,
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
            voices: &teleprompt_voice::catalogue::NONE,
        }
    }

    /// Every backend this build ships, each built from its own slice of
    /// `settings` (`backends:` in `config_file`).
    pub fn backends(
        &self,
        settings: &BTreeMap<String, serde_json::Value>,
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
