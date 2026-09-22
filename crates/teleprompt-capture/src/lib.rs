//! Stage 5: running a scene and keeping what it showed.
//!
//! Everything before this decided *when* each thing happens; the renderer
//! turns that into a file. This is the stage that makes the file worth
//! watching — without it a build is a correctly-paced video of nothing,
//! every cue holding its slot with a slate.
//!
//! The organising idea is that **a scene is a session**. The cues of a
//! walkthrough continue one another — a running program, a selected row,
//! an open log — so a backend is not handed a cue, it is handed a session
//! and told which of its cues to keep. Handing it cues one at a time
//! would restart the program once per cue, which is six fresh shells in a
//! two-minute video.
//!
//! That is also why a cue that is already cached still *runs*. Its clip is
//! not needed; the screen it leaves behind is, because the next cue opens
//! on it.

pub mod mock;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use teleprompt_core::Hash;

/// One action cue, as the planner needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub id: String,
    pub scene: String,
    pub adapter: String,
    /// `session="…"`, naming a different run of the scene.
    pub session: Option<String>,
    /// What the clip will be filed under — the chain, not the cue hash.
    pub key: Hash,
    /// The adapter's own source for this cue, as re-timed and published.
    pub source: String,
    /// How long the schedule gave this cue. A backend may not take
    /// longer; whether it may take less is the backend's business, since
    /// the renderer holds the last frame either way.
    pub duration_ms: u64,
    /// The scene's own settings, flattened to strings — what terminal to
    /// open, how big, in what shell. Strings because a backend reads the
    /// handful of keys it understands and the planner reads none of them;
    /// the same settings are in the capture key, so changing one
    /// invalidates the clips it changed.
    pub settings: BTreeMap<String, String>,
}

/// One run of a scene: the cues that share a screen, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub scene: String,
    pub adapter: String,
    pub name: Option<String>,
    /// The scene's settings, as every cue in it shares them.
    pub settings: BTreeMap<String, String>,
    pub cues: Vec<SessionCue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCue {
    pub id: String,
    pub key: Hash,
    pub source: String,
    pub duration_ms: u64,
    /// Whether a clip is wanted for this cue.
    ///
    /// `false` where one is already cached. The cue still runs — the next
    /// cue opens on the screen it leaves behind — it just is not kept.
    pub wanted: bool,
}

impl Session {
    /// How many of this session's cues are to be kept.
    pub fn wanted(&self) -> usize {
        self.cues.iter().filter(|s| s.wanted).count()
    }

    /// A setting, or what to use when the scene does not name one.
    pub fn setting<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.settings.get(key).map_or(default, String::as_str)
    }

    /// The settings under one nested key, as `prefix.name` pairs are
    /// flattened — `env:` being the one that matters.
    pub fn nested<'a>(&'a self, prefix: &str) -> impl Iterator<Item = (&'a str, &'a str)> {
        let prefix = format!("{prefix}.");
        self.settings
            .iter()
            .filter_map(move |(k, v)| k.strip_prefix(&prefix).map(|name| (name, v.as_str())))
    }

    /// A numeric setting, or the default when it is absent or unreadable.
    pub fn number(&self, key: &str, default: u32) -> u32 {
        self.settings
            .get(key)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }
}

/// The scene name a pause wears, which is not a scene and has no picture.
const PAUSE: &str = "pause";

/// Group `cues` into the sessions a backend can run, dropping what is
/// already captured.
///
/// `have` answers whether a clip for a key is in hand. It is a predicate
/// rather than a directory because the planner has no business reading the
/// filesystem, and a test has no business writing to one.
pub fn sessions(cues: &[Cue], have: &dyn Fn(&Hash) -> bool) -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();

    for cue in cues {
        // A pause has no picture: it holds whatever is on screen. There is
        // nothing to run and nothing to keep.
        if cue.scene == PAUSE {
            continue;
        }
        let in_session = SessionCue {
            id: cue.id.clone(),
            key: cue.key,
            source: cue.source.clone(),
            duration_ms: cue.duration_ms,
            wanted: !have(&cue.key),
        };
        match out
            .iter_mut()
            .find(|s| s.scene == cue.scene && s.name == cue.session)
        {
            Some(session) => session.cues.push(in_session),
            None => out.push(Session {
                scene: cue.scene.clone(),
                adapter: cue.adapter.clone(),
                name: cue.session.clone(),
                settings: cue.settings.clone(),
                cues: vec![in_session],
            }),
        }
    }

    for session in &mut out {
        // Steps after the last wanted one are not run. The prefix has to
        // be replayed to reach the screen a wanted cue opens on; the
        // suffix leads nowhere anybody is looking, and on a tape whose
        // sleeps are real seconds that is the difference between a capture
        // that stops early and one that sits there.
        if let Some(last) = session.cues.iter().rposition(|s| s.wanted) {
            session.cues.truncate(last + 1);
        }
    }
    // A session with nothing to keep is a session with nothing to do.
    out.retain(|s| s.wanted() > 0);
    out
}

/// A clip a backend produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clip {
    pub key: Hash,
    pub path: PathBuf,
}

/// The shape a capture has to fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// How far a capture has got. Reported per cue, because a cue is the
/// unit an author recognises and a session can be minutes long.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub scene: String,
    pub cue: String,
    pub done: usize,
    pub of: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The backend cannot run here — nothing to run a tape with, no
    /// terminal, no display.
    /// Not a failure of the script, and `build` turns it into a slate and
    /// a warning rather than an error.
    #[error("{backend} cannot capture here: {reason}")]
    Unavailable { backend: String, reason: String },
    #[error("{backend} failed to capture `{cue}`: {reason}")]
    Failed {
        backend: String,
        cue: String,
        reason: String,
    },
    #[error("cannot write {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// What can run a scene.
pub trait CaptureBackend {
    /// Stable identifier, for reporting which path a capture took.
    fn id(&self) -> &'static str;

    /// The scene adapter whose cues this backend speaks.
    fn adapter(&self) -> &'static str;

    /// Why this backend cannot run on this machine, if it cannot.
    ///
    /// Asked before anything is run, so `build` can say "no clips for
    /// `terminal`, and here is why" instead of failing halfway through a
    /// session that was never going to work.
    fn unavailable(&self) -> Option<String> {
        None
    }

    /// Run `session`, writing a clip into `out_dir` for each wanted cue.
    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Clip>, CaptureError>;
}

/// The backends a build can choose between.
#[derive(Default)]
pub struct CaptureRegistry {
    backends: Vec<Box<dyn CaptureBackend>>,
}

impl CaptureRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, backend: Box<dyn CaptureBackend>) -> Self {
        self.backends.push(backend);
        self
    }

    /// The backend for an adapter, or `None` where this build records
    /// nothing of that kind — which is a different answer from "it does,
    /// but not on this machine", and the caller reports them differently.
    pub fn for_adapter(&self, adapter: &str) -> Option<&dyn CaptureBackend> {
        self.backends
            .iter()
            .map(|b| b.as_ref())
            .find(|b| b.adapter() == adapter)
    }

    pub fn ids(&self) -> Vec<&'static str> {
        self.backends.iter().map(|b| b.id()).collect()
    }

    pub fn backends(&self) -> impl Iterator<Item = &dyn CaptureBackend> {
        self.backends.iter().map(|b| b.as_ref())
    }
}
