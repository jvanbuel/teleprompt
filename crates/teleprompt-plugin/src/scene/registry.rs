use std::collections::BTreeMap;

use super::contract::SceneCompiler;
use super::mock::MockScene;

#[derive(Default)]
pub struct SceneRegistry {
    plugins: BTreeMap<&'static str, Box<dyn SceneCompiler>>,
}

impl SceneRegistry {
    /// Only the mock: scene plugin crates depend on this one, so they are
    /// registered by the CLI (`teleprompt_cli::scene::scenes`).
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        r.register(Box::new(MockScene));
        r
    }

    pub fn register(&mut self, plugin: Box<dyn SceneCompiler>) {
        self.plugins.insert(plugin.kind(), plugin);
    }

    pub fn get(&self, plugin: &str) -> Option<&dyn SceneCompiler> {
        self.plugins.get(plugin).map(AsRef::as_ref)
    }

    pub fn available(&self) -> Vec<&'static str> {
        self.plugins.keys().copied().collect()
    }
}
