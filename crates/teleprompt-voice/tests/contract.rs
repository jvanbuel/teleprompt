//! What the contract offers someone implementing it from outside.

use teleprompt_voice::async_trait;
use teleprompt_voice::{
    LanguageSupport, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities, VoiceError,
};

/// The shape a network backend wants: one crate serving several configured
/// endpoints, so the id is built at construction rather than being a
/// literal. `capabilities().version` folds the endpoint in, because that is
/// the only handle a backend has on its own cache key.
struct ConfiguredVoice {
    id: String,
    endpoint: String,
}

#[async_trait]
impl VoiceBackend for ConfiguredVoice {
    fn id(&self) -> &str {
        &self.id
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            languages: LanguageSupport::Enumerated(vec!["en".to_string()]),
            cloning: false,
            cross_lingual: false,
            word_timings: false,
            ssml: false,
            speed_control: false,
            version: format!("1.0@{}", self.endpoint),
        }
    }

    async fn synthesize(&self, _req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        // The point of the test: a backend must be able to name *itself* in
        // its own error. With `backend: &'static str` this line did not
        // compile, because `id()` returns `&str`.
        Err(VoiceError::Unsupported {
            backend: self.id().to_string(),
            what: "word timings".to_string(),
        })
    }
}

fn backend(endpoint: &str) -> ConfiguredVoice {
    ConfiguredVoice {
        id: format!("configured-{endpoint}"),
        endpoint: endpoint.to_string(),
    }
}

#[test]
fn a_backend_whose_id_is_not_a_literal_can_name_itself_in_unsupported() {
    let b = backend("alpha");
    let e = VoiceError::Unsupported {
        backend: b.id().to_string(),
        what: "cloning".to_string(),
    };
    let rendered = e.to_string();
    assert!(rendered.contains("configured-alpha"), "{rendered}");
    assert!(rendered.contains("cloning"), "{rendered}");
}

/// The escape hatch documented on `VoiceCapabilities::version`: two
/// instances of one crate pointed at different servers must not share cache
/// entries, and folding the configuration into `version` is the only way
/// they can avoid it.
#[test]
fn folding_configuration_into_version_separates_two_instances() {
    assert_ne!(
        backend("alpha").capabilities().version,
        backend("beta").capabilities().version,
        "two differently-configured instances that share a version share \
         cache entries, and serve one voice's audio under the other's name"
    );
}
