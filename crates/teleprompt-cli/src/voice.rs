use std::sync::Arc;

use teleprompt_voice::{VoiceBackend, VoiceRegistry};
use teleprompt_voice_null::NullVoice;

/// The backends this build ships. Adding one is a line here plus a crate —
/// nothing else in the workspace changes.
pub fn registry() -> VoiceRegistry {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(NullVoice::default()));
    r
}

pub fn resolve(registry: &VoiceRegistry, id: &str) -> Result<Arc<dyn VoiceBackend>, String> {
    registry.get(id).ok_or_else(|| {
        format!(
            "unknown voice backend `{id}` (available: {})",
            registry.available().join(", ")
        )
    })
}
