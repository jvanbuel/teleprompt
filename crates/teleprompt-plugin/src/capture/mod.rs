//! Running a scene and keeping what it showed (`docs/design.md#capture`).
//!
//! A backend runs a session, not a shot: a walkthrough's shots continue one
//! another, so even a cached shot runs, for the screen the next opens on.

mod job;
pub mod mock;
pub mod reel;

pub use job::{absolute, Job};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use teleprompt_core::{Hash, ShotId};

/// One action shot, as the planner needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedShot {
    pub id: ShotId,
    pub scene: String,
    /// The scene plugin that records it.
    pub plugin: String,
    /// `session="…"`, naming a different run of the scene.
    pub session: Option<String>,
    /// What the clip is filed under: the chain, not the shot hash
    /// (`docs/design.md#capture-key`).
    pub key: Hash,
    /// The plugin's own source for this shot, as re-timed and published.
    pub source: String,
    /// How long the schedule gave this shot. A backend may not take longer;
    /// if it takes less, the renderer holds the last frame.
    pub duration_ms: u64,
    /// The scene's settings as strings, for a backend to read the keys it
    /// knows. Part of the capture key: changing one recaptures its clips.
    pub settings: BTreeMap<String, String>,
}

/// One run of a scene: the shots that share a screen, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub scene: String,
    #[serde(default)]
    pub plugin: String,
    #[serde(default)]
    pub name: Option<String>,
    /// The scene's settings, as every shot in it shares them.
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    /// What a relative path in `settings` is relative to: the project's
    /// directory. Empty, they are relative to where teleprompt runs.
    #[serde(default)]
    pub root: PathBuf,
    pub shots: Vec<SessionShot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionShot {
    pub id: ShotId,
    pub key: Hash,
    pub source: String,
    pub duration_ms: u64,
    /// Whether a clip is wanted: `false` where one is cached. It runs anyway.
    pub wanted: bool,
}

impl Session {
    /// How many of this session's shots are to be kept.
    pub fn wanted(&self) -> usize {
        self.shots.iter().filter(|s| s.wanted).count()
    }

    /// The setting `key` as a path, or `default`, against [`Self::root`].
    pub fn path(&self, key: &str, default: &str) -> PathBuf {
        self.root.join(self.setting(key, default))
    }

    /// A setting, or what to use when the scene does not name one.
    pub fn setting<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.settings.get(key).map_or(default, String::as_str)
    }

    /// The `prefix.name` settings as `(name, value)` pairs, such as `env`.
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
/// captured: `have`, a predicate, so the planner reads no filesystem.
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
                plugin: shot.plugin.clone(),
                name: shot.session.clone(),
                settings: shot.settings.clone(),
                root: PathBuf::new(),
                shots: vec![in_session],
            }),
        }
    }

    for session in &mut out {
        // Shots after the last wanted one lead nowhere: not run.
        if let Some(last) = session.shots.iter().rposition(|s| s.wanted) {
            session.shots.truncate(last + 1);
        }
    }
    out.retain(|s| s.wanted() > 0);
    out
}

/// A clip a backend produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clip {
    pub key: Hash,
    pub path: PathBuf,
}

/// The shape a capture has to fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    #[serde(default)]
    pub scene: String,
    pub shot: ShotId,
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
        shot: ShotId,
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
pub trait CaptureBackend: Send + Sync {
    /// Why it cannot run here, if it cannot: asked before a session starts.
    /// By default, the programs in [`Self::needs`] that are not on PATH.
    fn unavailable(&self) -> Option<String> {
        teleprompt_core::tool::missing_of(self.needs())
    }

    /// What it runs, which `teleprompt setup` lists and installs.
    fn needs(&self) -> &'static [&'static teleprompt_core::tool::Tool] {
        static NEEDS: &[&teleprompt_core::tool::Tool] = &[];
        NEEDS
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
