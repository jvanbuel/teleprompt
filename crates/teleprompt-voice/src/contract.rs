use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageSupport {
    Any,
    Enumerated(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceCapabilities {
    pub languages: LanguageSupport,
    pub cloning: bool,
    pub cross_lingual: bool,
    pub word_timings: bool,
    pub ssml: bool,
    pub speed_control: bool,
    /// The backend's own version, not teleprompt's. Feeds
    /// `teleprompt_cache::key`'s `backend_version`, which is what makes a
    /// backend release turn over its cache entries instead of teleprompt's
    /// own version doing that job for every backend at once.
    ///
    /// **This is also the only handle a backend has on its own cache key.**
    /// `teleprompt_cache::key` is built from `backend_id`, `backend_version`
    /// and the `SynthRequest` — text, locale, voice, speed — and nothing
    /// else. A backend whose output depends on configuration the request
    /// does not carry (a model checkpoint, a `base_url`, a sample rate, a
    /// vocoder setting) **must** fold that configuration into this string,
    /// or two differently-configured instances share cache entries.
    ///
    /// The failure mode of getting this wrong is not a stale build: it is
    /// serving one voice's audio under another voice's name, permanently,
    /// with `plan` reporting it as `measured` and nothing above this layer
    /// able to detect it. Version strings like
    /// `format!("{model}-{revision}@{host}")` are the intended shape.
    pub version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynthRequest {
    pub text: String,
    pub locale: String,
    pub voice: Option<String>,
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordTiming {
    pub word: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// What a backend produces. Duration is deliberately absent: it is
/// `pcm.duration_ms()`, so a backend cannot report a length its samples do
/// not have. An earlier contract returned the two separately, and `dub`
/// published a 3250 ms duration beside a 6500 ms file.
#[derive(Debug, Clone)]
pub struct Synthesized {
    pub pcm: Pcm,
    /// Present only when `capabilities().word_timings` is true.
    pub word_timings: Option<Vec<WordTiming>>,
}

/// Interleaved 16-bit PCM. The one audio representation that crosses a
/// backend boundary — encoders live downstream of this type, not inside
/// backends, so every backend produces the same thing and only one place
/// knows about file formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl Pcm {
    /// Length in milliseconds, computed per *frame*: with two channels,
    /// two samples are one instant in time, not two.
    pub fn duration_ms(&self) -> u64 {
        let channels = self.channels.max(1) as u64;
        let rate = self.sample_rate.max(1) as u64;
        let frames = self.samples.len() as u64 / channels;
        frames * 1000 / rate
    }
}

/// What went wrong, and whether trying again could help.
///
/// `backend` is a `String`, not a `&'static str`: `id()` returns `&str`, so
/// a backend whose id is not a literal — one crate serving several
/// configured endpoints, which is the shape a network backend wants — could
/// not name itself in its own error. One allocation on an error path is the
/// whole cost.
///
/// This was an enum until a backend arrived that could fail in ways worth
/// telling apart. The enum carried `backend` per-variant, so each new
/// variant re-litigated whether to include it and `Other(String)` simply
/// lost it — which is why Kokoro's client hand-formatted its base URL into a
/// bare string. Those are fields now, so a caller can render a failure
/// according to what it is doing instead of every caller printing one
/// pre-baked sentence.
#[derive(Debug, thiserror::Error)]
#[error("{backend}: {detail}")]
pub struct VoiceError {
    pub backend: String,
    pub kind: ErrorKind,
    /// Already scoped to the backend by the `Display` impl above, so it must
    /// not repeat the backend's name.
    pub detail: String,
    /// Only ever `Some` on `RateLimited`, and only when the server said so.
    pub retry_after: Option<std::time::Duration>,
}

/// How a failure should be treated. Deliberately about *treatment* rather
/// than about HTTP: a backend that speaks no HTTP still classifies into
/// these.
///
/// `#[non_exhaustive]` because the next backend will bring a kind nobody
/// predicted, and that should not be a breaking change for a workspace where
/// most callers only ever ask `retryable()`. The cost is real and worth
/// stating: a `match` that should have grown an arm falls through to its
/// wildcard instead of failing to compile, so every wildcard arm over
/// `ErrorKind` must be written to be correct for a kind it has never seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Credentials missing, malformed, or rejected.
    Auth,
    /// Credentials fine, allowance spent. Distinct from `RateLimited`
    /// because waiting does not help.
    Quota,
    /// Too many requests, or too many at once.
    RateLimited,
    /// The server rejected what we sent.
    ///
    /// Distinct from `Unsupported`: both mean the author must change
    /// something, but only this one proves a server was reached and a
    /// credential accepted — the first thing worth knowing when a `dub`
    /// fails.
    InvalidRequest,
    /// DNS, TLS, connection refused, timeout, 5xx.
    Transient,
    /// A 2xx whose body was not what the protocol promised.
    Protocol,
    /// The request asks for something this backend cannot do, decided
    /// locally without making a request.
    Unsupported,
    /// A bug in the backend itself.
    Internal,
}

impl ErrorKind {
    /// Whether trying the same request again could plausibly succeed.
    ///
    /// This hangs off `ErrorKind` rather than `VoiceError` on purpose.
    /// Retryability is a property of the classification, and putting it on
    /// the error would let two backends disagree about whether a 429 is
    /// worth retrying — a disagreement that stays invisible until one of
    /// them wastes an author's afternoon.
    pub fn retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Transient)
    }
}

impl VoiceError {
    pub fn new(backend: impl Into<String>, kind: ErrorKind, detail: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            kind,
            detail: detail.into(),
            retry_after: None,
        }
    }

    /// The request asks for something this backend cannot do.
    pub fn unsupported(backend: impl Into<String>, what: impl std::fmt::Display) -> Self {
        Self::new(
            backend,
            ErrorKind::Unsupported,
            format!("does not support {what}"),
        )
    }

    pub fn with_retry_after(mut self, after: std::time::Duration) -> Self {
        self.retry_after = Some(after);
        self
    }
}

/// One method that does the work, plus the two that let a caller holding
/// only `dyn VoiceBackend` identify what it has.
///
/// There used to be a fourth method here — `as_any`, an escape hatch for
/// capabilities genuinely one backend's own, such as listing a server's
/// voices. It is gone: every caller that needed a backend-specific
/// capability turned out to already be holding (or able to hold) the
/// concrete type at the point it was constructed, so downcasting a trait
/// object back into it was buying nothing the config layer did not already
/// offer. See `teleprompt-cli`'s `Backends::kokoro` for where that capability
/// now lives instead. A second server-backed backend needing the same thing
/// is the signal that this contract should grow a real method for it —
/// deliberately, not by re-adding a downcast.
#[async_trait::async_trait]
pub trait VoiceBackend: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> VoiceCapabilities;

    /// Turn text into audio. The only thing a backend does.
    ///
    /// Caching and duration prediction are teleprompt's concerns and appear
    /// nowhere in this contract.
    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError>;
}
