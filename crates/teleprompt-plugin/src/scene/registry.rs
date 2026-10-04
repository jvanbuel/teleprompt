use std::collections::BTreeMap;

use super::contract::SceneCompiler;
use super::mock::MockScene;

#[derive(Default)]
pub struct SceneRegistry {
    adapters: BTreeMap<&'static str, Box<dyn SceneCompiler>>,
}

impl SceneRegistry {
    /// Only the mock: scene plugin crates depend on this one, so they are
    /// registered by the CLI (`teleprompt_cli::scene::scenes`).
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        r.register(Box::new(MockScene));
        r
    }

    pub fn register(&mut self, adapter: Box<dyn SceneCompiler>) {
        self.adapters.insert(adapter.kind(), adapter);
    }

    pub fn get(&self, adapter: &str) -> Option<&dyn SceneCompiler> {
        self.adapters.get(adapter).map(AsRef::as_ref)
    }

    pub fn available(&self) -> Vec<&'static str> {
        self.adapters.keys().copied().collect()
    }
}
