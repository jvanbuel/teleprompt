use teleprompt_core::{Diagnostic, Hash, SourceSpan};

#[derive(Debug, Clone)]
pub struct BlockSource {
    pub scene: String,
    pub body: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone)]
pub struct Validated {
    pub scene: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Span {
    pub id: String,
    pub source: String,
    pub hash: Hash,
    pub index: usize,
}

/// Duration of a span, in milliseconds.
///
/// `Exact` comes from a declarative language that states its own timing.
/// `Estimated` is a guess the compiler may improve by measuring.
/// `Unknown` means the adapter cannot say and a measuring pass is required
/// (M1 and later; M0 adapters never return it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measured {
    Exact(u64),
    Estimated(u64),
    Unknown,
}

impl Measured {
    pub fn duration_ms(&self) -> Option<u64> {
        match self {
            Measured::Exact(ms) | Measured::Estimated(ms) => Some(*ms),
            Measured::Unknown => None,
        }
    }

    pub fn source_label(&self) -> &'static str {
        match self {
            Measured::Exact(_) => "exact",
            Measured::Estimated(_) => "estimated",
            Measured::Unknown => "unknown",
        }
    }
}

pub trait SceneCompiler: Send + Sync {
    fn kind(&self) -> &'static str;
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>>;
    fn spans(&self, v: &Validated, block_id: &str) -> Result<Vec<Span>, Vec<Diagnostic>>;
    fn estimate(&self, span: &Span) -> Measured;
}
