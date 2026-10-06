use teleprompt_core::{BlockId, Diagnostic, Hash, ShotId, SourceSpan};

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
    /// Scene plugins route every body diagnostic through this.
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
    pub id: ShotId,
    pub source: String,
    pub hash: Hash,
    pub index: usize,
    /// How long it lasts: `Exact` when the source states its timing,
    /// `Estimated` for a bound, `Unknown` to take its line's length.
    pub length: Measured,
}

/// A shot's duration in milliseconds; see `docs/design.md#scene-contract`.
/// Written `{"ms": …, "exact": …}`: `ms` with `exact` when the source
/// states it, `ms` alone for an estimate, neither when it is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "Length", into = "Length")]
pub enum Measured {
    Exact(u64),
    Estimated(u64),
    Unknown,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Length {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ms: Option<u64>,
    #[serde(default)]
    exact: bool,
}

impl From<Length> for Measured {
    fn from(l: Length) -> Self {
        match (l.ms, l.exact) {
            (Some(ms), true) => Measured::Exact(ms),
            (Some(ms), false) => Measured::Estimated(ms),
            (None, _) => Measured::Unknown,
        }
    }
}

impl From<Measured> for Length {
    fn from(m: Measured) -> Self {
        match m {
            Measured::Exact(ms) => Length {
                ms: Some(ms),
                exact: true,
            },
            Measured::Estimated(ms) => Length {
                ms: Some(ms),
                exact: false,
            },
            Measured::Unknown => Length {
                ms: None,
                exact: false,
            },
        }
    }
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
    /// scene plugin understands, the nearest correct spelling).
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

impl From<&BlockSource> for Validated {
    /// A block accepted as written.
    fn from(src: &BlockSource) -> Self {
        Validated {
            scene: src.scene.clone(),
            body: src.body.clone(),
        }
    }
}

impl Shot {
    /// The `index`th shot of `block_id`, named `<block>#<index>`, lasting
    /// as long as its line until [`Self::lasting`] says otherwise. `hash` is
    /// the plugin's to choose: whatever identifies the shot's picture.
    pub fn numbered(block_id: &BlockId, index: usize, source: String, hash: Hash) -> Self {
        Shot {
            id: ShotId::of(block_id, index),
            source,
            hash,
            index,
            length: Measured::Unknown,
        }
    }

    /// The same shot, lasting `length`.
    pub fn lasting(self, length: Measured) -> Self {
        Shot { length, ..self }
    }
}

/// Whether a line says something: not blank, and not a `#` comment.
pub fn is_content(line: &str) -> bool {
    !line.trim().is_empty() && !line.trim_start().starts_with('#')
}

/// A body split at lines that are exactly `mark`: each part, with the
/// 0-based body line it starts on.
pub fn split_at_mark(body: &str, mark: &str) -> Vec<(usize, String)> {
    let mut out = vec![(0, String::new())];
    for (i, line) in body.lines().enumerate() {
        if line.trim() == mark {
            out.push((i + 1, String::new()));
        } else {
            let part = &mut out.last_mut().expect("seeded").1;
            part.push_str(line);
            part.push('\n');
        }
    }
    out
}

/// The parts of a marked body that `include=file#fragment` names: `2` is
/// the second part (after the first mark), `2-3` a range of them, kept
/// with the mark between so they stay two shots.
pub fn select_marked(body: &str, mark: &str, fragment: &str) -> Result<String, String> {
    let parts = split_at_mark(body, mark);
    let count = parts.len();
    let numbered = |s: &str| {
        s.trim()
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=count).contains(n))
    };
    let range = match fragment.split_once('-') {
        Some((a, b)) => numbered(a).zip(numbered(b)).filter(|(a, b)| a <= b),
        None => numbered(fragment).map(|n| (n, n)),
    };
    let (first, last) = range.ok_or_else(|| {
        format!("`#{fragment}` names no part of this file, which has {count} part(s) between its `{mark}` lines")
    })?;
    Ok(parts[first - 1..last]
        .iter()
        .map(|(_, part)| part.as_str())
        .collect::<Vec<_>>()
        .join(&format!("{mark}\n")))
}

/// Validates each part of a body between marks with `check`, reporting a
/// part's failure at its first line of content.
pub fn validate_parts(
    src: &BlockSource,
    mark: &str,
    check: impl Fn(&str) -> Result<(), String>,
) -> Result<Validated, Vec<Diagnostic>> {
    let diags: Vec<Diagnostic> = split_at_mark(&src.body, mark)
        .into_iter()
        .filter_map(|(first, part)| {
            let why = check(&part).err()?;
            let at = part.lines().position(is_content).unwrap_or(0);
            Some(src.origin.locate(Diagnostic::error(why), first + at, 0))
        })
        .collect();
    if diags.is_empty() {
        Ok(Validated::from(src))
    } else {
        Err(diags)
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
        Ok(Validated::from(src))
    } else {
        Err(diags)
    }
}

/// The compile-time half of a scene plugin: synchronous and free of IO.
/// Rationale for each method is in `docs/design.md#scene-contract`.
pub trait SceneCompiler: Send + Sync {
    fn kind(&self) -> &'static str;
    /// Report every bad line, positioned through [`BodyOrigin::locate`].
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>>;
    /// Split at marks, each shot with its [`Shot::length`]; a block
    /// without marks is one shot.
    fn shots(&self, v: &Validated, block_id: &BlockId) -> Result<Vec<Shot>, Vec<Diagnostic>>;

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
    /// is the plugin's. The default refuses, naming the plugin.
    fn select(&self, _body: &str, fragment: &str) -> Result<String, String> {
        Err(format!(
            "`{}` blocks do not take an `include=…#{fragment}`",
            self.kind()
        ))
    }
}
