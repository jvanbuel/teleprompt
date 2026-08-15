use teleprompt_core::{Diagnostic, Hash, SourceSpan};

/// Where an action block's body text actually lives.
///
/// A `BlockSource` used to carry only the *fence's* span in the script, and
/// adapters offset body line `i` by `span.line + i + 1`. That is right for a
/// body written inline, and meaningless for one loaded with `include=`: an
/// error on line 3 of `steps.mock` came out as `scripts/h1.md:12:1` in a
/// ten-line script. Controller ruling F12 removed a fabricated `line: 0` from
/// this exact path; `include=` (T16) reintroduced a fabricated line by
/// another route, ten tasks after F12 set the convention. An origin — which
/// file, starting at which line — is what the convention actually needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyOrigin {
    /// Written inline in the script, inside the fence at `fence`. Body line
    /// `i` (0-based) is script line `fence.line + i + 1`; the `+ 1` skips the
    /// opening ```` ``` ```` line itself.
    Inline { fence: SourceSpan },
    /// Loaded from another file. Body line `i` is line `i + 1` of `path`, and
    /// diagnostics name `path` rather than the script.
    Included { path: String },
}

impl BodyOrigin {
    /// Source location of 0-based body line `i`, `len` characters wide.
    pub fn span_of(&self, i: usize, len: usize) -> SourceSpan {
        let line = match self {
            BodyOrigin::Inline { fence } => fence.line + i + 1,
            BodyOrigin::Included { .. } => i + 1,
        };
        SourceSpan {
            line,
            column: 1,
            len,
        }
    }

    /// The file diagnostics about this body should name, or `None` when the
    /// script the caller is already rendering is the right answer.
    pub fn file(&self) -> Option<&str> {
        match self {
            BodyOrigin::Inline { .. } => None,
            BodyOrigin::Included { path } => Some(path),
        }
    }

    /// Point `d` at 0-based body line `i`, in whichever file that line is
    /// really in. Adapters should route every body diagnostic through this
    /// rather than doing their own arithmetic.
    pub fn locate(&self, d: Diagnostic, i: usize, len: usize) -> Diagnostic {
        let d = d.at(self.span_of(i, len));
        match self.file() {
            Some(path) => d.in_file(path),
            None => d,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BlockSource {
    pub scene: String,
    pub body: String,
    pub origin: BodyOrigin,
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
