use teleprompt_core::{Diagnostic, Hash, SourceSpan};

/// Where an action block's body text actually lives.
///
/// A `BlockSource` used to carry only the *fence's* shot in the script, and
/// adapters offset body line `i` by `shot.line + i + 1`. That is right for a
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
pub struct Shot {
    pub id: String,
    pub source: String,
    pub hash: Hash,
    pub index: usize,
}

/// Duration of a shot, in milliseconds.
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

/// One uninterpretable line, as its adapter's line classifier saw it.
///
/// Deliberately not an `Error`: this never propagates as one. It is the half
/// of a diagnostic an adapter knows — what is wrong and how to fix it — with
/// the half only the caller knows, the location, left out. [`validate_commands`]
/// joins the two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandError {
    pub message: String,
    /// Owned rather than `&'static str`: the most useful help an adapter can
    /// give is often computed — the list of directives it understands, or the
    /// correct spelling of the one that was nearly right — and a borrowed
    /// help line forces those to be dropped or leaked.
    pub help: Option<String>,
}

impl CommandError {
    /// A line error with a help line attached.
    pub fn new(message: impl Into<String>, help: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            help: Some(help.into()),
        }
    }

    /// A line error with nothing useful to suggest.
    pub fn bare(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            help: None,
        }
    }
}

/// Validate a body one line at a time, reporting every bad line.
///
/// Adapters whose language is line-oriented implement `classify` and call
/// this rather than writing the loop again. Two things it gets right that a
/// hand-written loop has twice gotten wrong: it reports *all* failures rather
/// than the first, so one `check` fixes a whole block; and it routes every
/// diagnostic through [`BodyOrigin::locate`], so an `include`d body names its
/// own file at its own line numbers instead of an offset into the script.
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

pub trait SceneCompiler: Send + Sync {
    fn kind(&self) -> &'static str;
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>>;
    fn shots(&self, v: &Validated, block_id: &str) -> Result<Vec<Shot>, Vec<Diagnostic>>;
    fn estimate(&self, shot: &Shot) -> Measured;

    /// The same shot, re-written to last `target_ms`, or `None` when this
    /// adapter cannot promise that.
    ///
    /// This is the other half of `stretch-action` and `trim-action`. The
    /// scheduler decides how long an action *should* take — usually as long
    /// as the sentence over it — but the number alone changes nothing about
    /// what a capture records: the tape still runs at its authored pace, and
    /// whatever is left of the slot is a frozen frame. Re-timing puts the
    /// scheduler's decision back into the adapter's own language, so what is
    /// captured lasts as long as the schedule says.
    ///
    /// Returning `None` is the honest answer wherever the source does not
    /// state its own timing — a tape that waits for a prompt is as long as
    /// the command takes, and no arithmetic here changes that. The default
    /// is `None`, so an adapter that cannot re-time says nothing and the
    /// renderer holds a frame instead.
    fn retime(&self, _span: &Shot, _target_ms: u64) -> Option<String> {
        None
    }

    /// Whether a shot opens on the screen the shot before it left behind.
    ///
    /// True of a terminal and a browser — a running program, a selected
    /// row, a signed-in page — and so true by default: a shot's picture is
    /// then named by its own source *and every source before it* in its
    /// session, and editing one re-captures everything after it.
    ///
    /// An adapter whose shots are functions of their own source alone —
    /// a composition that draws the same frames whatever preceded it —
    /// returns `false`, and each of its shots is named by itself. Editing
    /// one then re-captures that one, which is the whole point of saying
    /// so: chaining a stateless scene is correct and needlessly expensive.
    fn continues(&self) -> bool {
        true
    }

    /// Files outside the block that this scene's picture is drawn from.
    ///
    /// A tape is the whole of what a terminal shows, so the default is
    /// none. A scene drawn by a project of its own — a component library,
    /// a stylesheet — names that project's files here, and the compiler
    /// hashes their contents into every shot's capture key, so editing one
    /// re-captures what it draws. Paths only: the adapter names them, the
    /// compiler reads them, and `SceneCompiler` stays free of IO. A
    /// directory is read recursively, skipping `node_modules` and entries
    /// whose names begin with `.`; a path that does not exist is skipped.
    fn inputs(&self, _scene: &teleprompt_core::config::SceneConfig) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
}
