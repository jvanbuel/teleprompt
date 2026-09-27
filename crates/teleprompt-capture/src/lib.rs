//! Stage 5: running a scene and keeping what it showed
//! (`docs/design.md#capture`).
//!
//! A backend is handed a session, not a shot, and told which shots to keep:
//! the shots of a walkthrough continue one another, so running them one at
//! a time would restart the program per shot. That is also why a cached
//! shot still runs: the next shot opens on the screen it leaves.

pub mod mock;
pub mod record;
pub mod reel;
pub mod tool;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use teleprompt_core::Hash;

/// One action shot, as the planner needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedShot {
    pub id: String,
    pub scene: String,
    pub adapter: String,
    /// `session="…"`, naming a different run of the scene.
    pub session: Option<String>,
    /// What the clip is filed under: the chain, not the shot hash
    /// (`docs/design.md#capture-key`).
    pub key: Hash,
    /// The adapter's own source for this shot, as re-timed and published.
    pub source: String,
    /// How long the schedule gave this shot. A backend may not take longer;
    /// if it takes less, the renderer holds the last frame.
    pub duration_ms: u64,
    /// The scene's settings, flattened to strings: a backend reads the keys
    /// it understands and the planner reads none. They are part of the
    /// capture key, so changing one invalidates the clips it affects.
    pub settings: BTreeMap<String, String>,
}

/// One run of a scene: the shots that share a screen, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub scene: String,
    pub adapter: String,
    pub name: Option<String>,
    /// The scene's settings, as every shot in it shares them.
    pub settings: BTreeMap<String, String>,
    pub shots: Vec<SessionShot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionShot {
    pub id: String,
    pub key: Hash,
    pub source: String,
    pub duration_ms: u64,
    /// Whether a clip is wanted: `false` where one is already cached. The
    /// shot still runs either way.
    pub wanted: bool,
}

impl Session {
    /// How many of this session's shots are to be kept.
    pub fn wanted(&self) -> usize {
        self.shots.iter().filter(|s| s.wanted).count()
    }

    /// A setting, or what to use when the scene does not name one.
    pub fn setting<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.settings.get(key).map_or(default, String::as_str)
    }

    /// The `prefix.name` settings as `(name, value)` pairs, such as the
    /// scene's `env`.
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

/// Group `shots` into the sessions a backend can run, dropping what is
/// already captured. `have` is a predicate rather than a directory, so the
/// planner reads no filesystem and a test writes none.
pub fn sessions(shots: &[PlannedShot], have: &dyn Fn(&Hash) -> bool) -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();

    for shot in shots {
        // A pause has no picture: nothing to run and nothing to keep.
        if shot.scene == PAUSE {
            continue;
        }
        let in_session = SessionShot {
            id: shot.id.clone(),
            key: shot.key,
            source: shot.source.clone(),
            duration_ms: shot.duration_ms,
            wanted: !have(&shot.key),
        };
        match out
            .iter_mut()
            .find(|s| s.scene == shot.scene && s.name == shot.session)
        {
            Some(session) => session.shots.push(in_session),
            None => out.push(Session {
                scene: shot.scene.clone(),
                adapter: shot.adapter.clone(),
                name: shot.session.clone(),
                settings: shot.settings.clone(),
                shots: vec![in_session],
            }),
        }
    }

    for session in &mut out {
        // Shots after the last wanted one are not run: they lead nowhere
        // anyone is looking, and a tape's sleeps are real seconds.
        if let Some(last) = session.shots.iter().rposition(|s| s.wanted) {
            session.shots.truncate(last + 1);
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

/// A scratch directory a backend records into, removed when dropped, so a
/// failed capture leaves no half-made recording among the clips.
#[derive(Debug)]
pub struct WorkDir(PathBuf);

impl WorkDir {
    /// Creates `parent/.{name}-{pid}`.
    pub fn create(parent: &Path, name: &str) -> std::io::Result<WorkDir> {
        let path = parent.join(format!(".{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(WorkDir(path))
    }
}

impl std::ops::Deref for WorkDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for WorkDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// How far a capture has got, per shot: the unit an author recognises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub scene: String,
    pub shot: String,
    pub done: usize,
    pub of: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The backend cannot run here (no tool, terminal or display). Not a
    /// script error: `build` renders its shots as slates, with a warning.
    #[error("{backend} cannot capture here: {reason}")]
    Unavailable { backend: String, reason: String },
    #[error("{backend} failed to capture `{shot}`: {reason}")]
    Failed {
        backend: String,
        shot: String,
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
    /// The scene adapter whose shots this backend speaks.
    fn adapter(&self) -> &'static str;

    /// Why this backend cannot run on this machine, if it cannot. Asked
    /// first, so `build` reports it instead of failing mid-session.
    fn unavailable(&self) -> Option<String> {
        None
    }

    /// Run `session`, writing a clip into `out_dir` for each wanted shot.
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

    /// The backend for an adapter. `None` where this build records nothing of
    /// that kind, which the caller reports differently from
    /// [`CaptureBackend::unavailable`].
    pub fn for_adapter(&self, adapter: &str) -> Option<&dyn CaptureBackend> {
        self.backends
            .iter()
            .map(|b| b.as_ref())
            .find(|b| b.adapter() == adapter)
    }

    pub fn backends(&self) -> impl Iterator<Item = &dyn CaptureBackend> {
        self.backends.iter().map(|b| b.as_ref())
    }
}
