use std::collections::BTreeMap;
use std::sync::Arc;

use crate::contract::VoiceBackend;

/// Mirrors `SceneRegistry`. `BTreeMap` so `available()` is ordered and
/// `doctor`'s output does not depend on insertion order.
///
/// `Arc`, not `Box`: `dub` synthesizes segments concurrently, so the backend
/// is shared across tasks.
#[derive(Default)]
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

/// Manual, not derived: `Arc<dyn VoiceBackend>` has no `Debug` impl (the
/// trait carries none — a backend's internals are none of the registry's
/// business), so a `#[derive(Debug)]` here could not compile. The
/// registered ids are what a caller actually wants to see, e.g. in an
/// `unwrap_err`/`expect` panic message when a `Result<VoiceRegistry, _>`
/// needs printing.
impl std::fmt::Debug for VoiceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceRegistry")
            .field("backends", &self.available())
            .finish()
    }
}
