use teleprompt_core::{Diagnostic, Hash, SourceSpan};

/// Where an action block's body lives, so diagnostics point at the right file
/// and line: an `include=`d body's line 3 is line 3 of that file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyOrigin {
    /// Inside the fence at `fence`; body line `i` (0-based) is script line
    /// `fence.line + i + 1`, skipping the opening fence line.
    Inline { fence: SourceSpan },
    /// Body line `i` is line `i + 1` of `path`.
    Included { path: String },
}

impl BodyOrigin {
    pub(crate) fn span_of(&self, i: usize, len: usize) -> SourceSpan {
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

    /// The file to name in diagnostics, or `None` for the script itself.
    pub fn file(&self) -> Option<&str> {
        match self {
            BodyOrigin::Inline { .. } => None,
            BodyOrigin::Included { path } => Some(path),
        }
    }

    /// Point `d` at 0-based body line `i` in whichever file holds it.
    /// Adapters route every body diagnostic through this.
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
pub struct Shot {
    pub id: String,
    pub source: String,
    pub hash: Hash,
    pub index: usize,
}

/// A shot's duration in milliseconds; see `docs/design.md#scene-contract`.
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
}

/// What is wrong with one body line, without its location, which
/// [`validate_commands`] adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandError {
    pub message: String,
    /// Owned, because useful help is often computed (the directives an
    /// adapter understands, the nearest correct spelling).
    pub help: Option<String>,
}

impl CommandError {
    pub fn new(message: impl Into<String>, help: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            help: Some(help.into()),
        }
    }
}

/// Validate a line-oriented body with `classify`, reporting every bad line
/// (not just the first) positioned through [`BodyOrigin::locate`].
pub fn validate_commands<T>(
    src: &BlockSource,
    classify: impl Fn(&str) -> Result<T, CommandError>,
) -> Result<Validated, Vec<Diagnostic>> {
    let diags: Vec<Diagnostic> = src
        .body
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let e = classify(line).err()?;
            let mut d = Diagnostic::error(e.message);
            if let Some(h) = e.help {
                d = d.with_help(h);
            }
            Some(src.origin.locate(d, i, line.trim().len()))
        })
        .collect();

    if diags.is_empty() {
        Ok(Validated {
            scene: src.scene.clone(),
            body: src.body.clone(),
        })
    } else {
        Err(diags)
    }
}

/// The compile-time half of a scene adapter: synchronous and free of IO.
/// Rationale for each method is in `docs/design.md#scene-contract`.
pub trait SceneCompiler: Send + Sync {
    fn kind(&self) -> &'static str;
    /// Report every bad line, positioned through [`BodyOrigin::locate`].
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>>;
    /// Split at marks; a block without marks is one shot.
    fn shots(&self, v: &Validated, block_id: &str) -> Result<Vec<Shot>, Vec<Diagnostic>>;
    /// `Exact` when the source states its timing, `Estimated` for a bound,
    /// `Unknown` otherwise (the shot then takes its line's length).
    fn estimate(&self, shot: &Shot) -> Measured;

    /// The shot's source rewritten to last exactly `target_ms`, or `None`
    /// when the source does not state its own timing (a tape waiting on a
    /// prompt). With `None` the renderer holds the last frame for the slot.
    fn retime(&self, _span: &Shot, _target_ms: u64) -> Option<String> {
        None
    }

    /// Whether a shot opens on the screen the previous shot left, so its
    /// capture key chains every earlier shot in the session. Return `false`
    /// only when a shot's picture depends on its own source alone.
    fn continues(&self) -> bool {
        true
    }

    /// Files outside the block the scene's picture is drawn from, hashed into
    /// every shot's capture key. Return paths only; the compiler reads them,
    /// recursing into directories, skipping `node_modules` and entries
    /// starting with `.`, and ignoring paths that do not exist.
    fn inputs(&self, _scene: &teleprompt_core::config::SceneConfig) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    /// Files one shot is drawn from beyond [`inputs`] (a media shot's image),
    /// hashed into that shot's key alone. `source` is the shot's published
    /// source; paths only, as with `inputs`.
    ///
    /// [`inputs`]: SceneCompiler::inputs
    fn shot_inputs(
        &self,
        _scene: &teleprompt_core::config::SceneConfig,
        _source: &str,
    ) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    /// The part of an included body that `include=file#fragment` names. The
    /// compiler has already validated the whole file; the fragment's meaning
    /// is the adapter's. The default refuses, naming the adapter.
    fn select(&self, _body: &str, fragment: &str) -> Result<String, String> {
        Err(format!(
            "`{}` blocks do not take an `include=…#{fragment}`",
            self.kind()
        ))
    }
}
