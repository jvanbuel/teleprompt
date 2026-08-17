use std::collections::BTreeMap;

use crate::contract::SceneCompiler;
use crate::mock::MockScene;
use crate::vhs::VhsScene;

#[derive(Default)]
pub struct SceneRegistry {
    adapters: BTreeMap<&'static str, Box<dyn SceneCompiler>>,
}

impl SceneRegistry {
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        r.register(Box::new(MockScene));
        r.register(Box::new(VhsScene));
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
