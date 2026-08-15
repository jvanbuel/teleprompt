use teleprompt_core::Hash;

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
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynthRequest {
    pub text: String,
    pub locale: String,
    pub voice: Option<String>,
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordTiming {
    pub word: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SynthResult {
    pub duration_ms: u64,
    pub audio_hash: Hash,
    pub word_timings: Option<Vec<WordTiming>>,
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("backend `{backend}` does not support {what}")]
    Unsupported { backend: &'static str, what: String },
    #[error("{0}")]
    Other(String),
}

pub trait VoiceBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> VoiceCapabilities;
    fn synthesize(&self, req: &SynthRequest) -> Result<SynthResult, VoiceError>;
    fn cache_key(&self, req: &SynthRequest) -> String;
}
