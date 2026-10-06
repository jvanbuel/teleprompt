use std::collections::BTreeMap;
use std::sync::Arc;

use crate::VoiceBackend;

/// Ordered so `setup`'s output does not depend on insertion order. `Arc`
/// because `dub` shares a backend across concurrent synthesis tasks.
#[derive(Default, Clone)]
pub struct VoiceRegistry {
    backends: BTreeMap<String, Arc<dyn VoiceBackend>>,
}

impl VoiceRegistry {
    pub fn register(&mut self, backend: Arc<dyn VoiceBackend>) {
        self.backends.insert(backend.id().to_string(), backend);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn VoiceBackend>> {
        self.backends.get(id).cloned()
    }

    pub fn available(&self) -> Vec<&str> {
        self.backends.keys().map(String::as_str).collect()
    }
}
