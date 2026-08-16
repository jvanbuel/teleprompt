use std::collections::BTreeMap;
use std::sync::Arc;

use teleprompt_voice::{VoiceBackend, VoiceRegistry};
use teleprompt_voice_kokoro::{KokoroConfig, KokoroVoice};
use teleprompt_voice_null::NullVoice;

/// The backends this build ships, each constructed from its own slice of
/// `backends:`. Adding one is a line here plus a crate — nothing else in the
/// workspace changes, and in particular `teleprompt-core` never learns the
/// new backend's name.
pub fn registry_for(
    backends: &BTreeMap<String, serde_yaml::Value>,
) -> Result<VoiceRegistry, String> {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(NullVoice::default()));

    let kokoro = match backends.get("kokoro") {
        Some(v) => KokoroConfig::from_value(v)?,
        None => KokoroConfig::default(),
    };
    r.register(Arc::new(KokoroVoice::new(kokoro)?));

    Ok(r)
}

/// The default registry, for callers with no project config in hand
/// (`doctor` outside a project, tests). Settings-free, so every backend gets
/// its defaults.
///
/// The `expect` cannot fail from any configuration: an empty map always
/// takes `KokoroConfig::default()`, never `from_value`, so there is no
/// deserialization or validation `Err` this call could ever produce. The
/// one path still live underneath it is environmental, not configuration —
/// `KokoroVoice::new` also propagates a failure from
/// `reqwest::Client::builder().build()`, which can fail if the runtime's
/// TLS backend is broken. That is not proven impossible, only unreachable
/// from any input this function controls; a caller relying on this panic
/// never firing should read it as "no configuration can trigger this," not
/// "this cannot happen." If it ever does, the process could not have done
/// the work anyway, so the panic — outside `exit_code_for`'s vocabulary —
/// is at least an honest signal that something below `main` is broken.
pub fn registry() -> VoiceRegistry {
    registry_for(&BTreeMap::new()).expect("default backend settings are always valid")
}

pub fn resolve(registry: &VoiceRegistry, id: &str) -> Result<Arc<dyn VoiceBackend>, String> {
    registry.get(id).ok_or_else(|| {
        format!(
            "unknown voice backend `{id}` (available: {})",
            registry.available().join(", ")
        )
    })
}
