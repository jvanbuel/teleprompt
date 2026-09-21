use std::collections::BTreeMap;

use crate::contract::SceneCompiler;
use crate::mock::MockScene;

#[derive(Default)]
pub struct SceneRegistry {
    adapters: BTreeMap<&'static str, Box<dyn SceneCompiler>>,
}

impl SceneRegistry {
    /// The adapters this crate carries itself, which is the mock and nothing
    /// else.
    ///
    /// Not "every adapter teleprompt ships": an adapter crate depends on this
    /// one for the contract, so this one cannot depend back on it to register
    /// it. Assembling the full set is the composition root's job — see
    /// `teleprompt_cli::scene::scenes`, which starts from here and adds the
    /// rest, exactly as `teleprompt_cli::voice::backends_for` does for voice.
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
