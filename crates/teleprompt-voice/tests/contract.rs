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
            speed: None,
            version: format!("1.0@{}", self.endpoint),
        }
    }

    async fn synthesize(&self, _req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        // The point of the test: a backend must be able to name *itself* in
        // its own error. With `backend: &'static str` this line did not
        // compile, because `id()` returns `&str`.
        Err(VoiceError::unsupported(self.id(), "word timings"))
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
    let e = VoiceError::unsupported(b.id(), "cloning");
    let rendered = e.to_string();
    assert!(rendered.contains("configured-alpha"), "{rendered}");
    assert!(rendered.contains("cloning"), "{rendered}");
}

#[test]
fn error_kinds_classify_retryability() {
    use teleprompt_voice::ErrorKind;
    assert!(ErrorKind::RateLimited.retryable());
    assert!(ErrorKind::Transient.retryable());
    for k in [
        ErrorKind::Auth,
        ErrorKind::Quota,
        ErrorKind::InvalidRequest,
        ErrorKind::Protocol,
        ErrorKind::Unsupported,
        ErrorKind::Internal,
    ] {
        assert!(!k.retryable(), "{k:?} must not be retryable");
    }
}

/// `Display` prefixes the backend, so a `detail` that repeats it would say
/// the name twice. Every construction site depends on this.
#[test]
fn error_display_names_the_backend_once() {
    let e = VoiceError::new(
        "kokoro",
        teleprompt_voice::ErrorKind::Transient,
        "no response within 30000ms",
    );
    assert_eq!(e.to_string(), "kokoro: no response within 30000ms");
}

#[test]
fn retry_after_is_absent_unless_set() {
    let e = VoiceError::new("x", teleprompt_voice::ErrorKind::RateLimited, "slow down");
    assert!(e.retry_after.is_none());
    let e = e.with_retry_after(std::time::Duration::from_secs(2));
    assert_eq!(e.retry_after, Some(std::time::Duration::from_secs(2)));
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

/// A backend that claims word timings and does not deliver them.
struct LyingVoice;

#[async_trait]
impl VoiceBackend for LyingVoice {
    fn id(&self) -> &str {
        "lying"
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            languages: LanguageSupport::Any,
            cloning: false,
            cross_lingual: false,
            word_timings: true,
            ssml: false,
            speed: None,
            version: "lying-1".to_string(),
        }
    }

    async fn synthesize(&self, _req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        Ok(Synthesized {
            pcm: teleprompt_voice::Pcm {
                sample_rate: 48_000,
                channels: 1,
                samples: vec![0; 48],
            },
            word_timings: None,
        })
    }
}

/// A backend that keeps the promise.
struct TimedVoice;

#[async_trait]
impl VoiceBackend for TimedVoice {
    fn id(&self) -> &str {
        "timed"
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            word_timings: true,
            ..LyingVoice.capabilities()
        }
    }

    async fn synthesize(&self, _req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        Ok(Synthesized {
            word_timings: Some(vec![teleprompt_voice::WordTiming {
                word: "hello".to_string(),
                start_ms: 0,
                end_ms: 1,
            }]),
            ..LyingVoice.synthesize(_req).await.unwrap()
        })
    }
}

fn any_request() -> SynthRequest {
    SynthRequest {
        text: "hello".to_string(),
        locale: "en".to_string(),
        voice: None,
        speed: 1.0,
    }
}

/// The invariant `Synthesized`'s doc has asserted since M1 and nothing has
/// ever enforced, because nothing in the workspace reported `true`. The
/// ElevenLabs backend will, so it is pinned here first — a contract clause
/// that only starts being checked once something depends on it has already
/// had its chance to be wrong.
async fn assert_word_timings_invariant(b: &dyn VoiceBackend) {
    let out = b
        .synthesize(&any_request())
        .await
        .expect("synthesis succeeded");
    if b.capabilities().word_timings {
        assert!(
            out.word_timings.is_some(),
            "backend `{}` claims word_timings but returned None",
            b.id()
        );
    }
}

#[tokio::test]
#[should_panic(expected = "claims word_timings but returned None")]
async fn a_backend_that_claims_timings_must_produce_them() {
    assert_word_timings_invariant(&LyingVoice).await;
}

#[tokio::test]
async fn a_backend_that_delivers_timings_satisfies_the_invariant() {
    assert_word_timings_invariant(&TimedVoice).await;
}

/// And a backend that claims nothing is free to return nothing — the
/// invariant is one-directional, which is what lets `null` and `kokoro`
/// stay as they are.
#[tokio::test]
async fn a_backend_that_claims_no_timings_may_return_none() {
    struct Quiet;
    #[async_trait]
    impl VoiceBackend for Quiet {
        fn id(&self) -> &str {
            "quiet"
        }
        fn capabilities(&self) -> VoiceCapabilities {
            VoiceCapabilities {
                word_timings: false,
                ..LyingVoice.capabilities()
            }
        }
        async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
            LyingVoice.synthesize(req).await
        }
    }
    assert_word_timings_invariant(&Quiet).await;
}
