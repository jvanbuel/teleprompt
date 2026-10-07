mod compile;

pub use compile::{translation_path, Compiled, Script};

use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use teleprompt_script::config::PartialConfig;

use crate::registry::Registry;

/// A discovered teleprompt project: the directory containing
/// `teleprompt.toml` and that file's parsed contents.
///
/// `Clone` because the prompter's voice and its threads each keep one.
#[derive(Clone)]
pub struct Project {
    pub root: PathBuf,
    pub config: PartialConfig,
    /// What this build has to compile and record it with.
    pub registry: Registry,
}

/// The directory a script lives in. `Path::new("demo.md").parent()` is
/// `Some("")`, not `None`, and `""` does not canonicalize, so the bare
/// filename case needs `.` spelled out.
pub fn script_dir(script: &Path) -> &Path {
    match script.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// Where a project's derived artifacts live (docs/design.md#caches).
#[derive(Debug, Clone)]
pub struct CacheDirs {
    pub root: PathBuf,
}

impl CacheDirs {
    /// The caches of the project rooted at `project_root`.
    pub fn under(project_root: &Path) -> Self {
        Self {
            root: project_root.join(".teleprompt").join("cache"),
        }
    }

    /// Synthesized lines.
    pub fn voice(&self) -> CacheDir<Voice> {
        CacheDir::at(self.root.join("voice"))
    }

    /// Captured clips, by capture key.
    pub fn clips(&self) -> CacheDir<Clips> {
        CacheDir::at(self.root.join("video"))
    }

    /// Encoded chunks of rendered picture.
    pub fn compose(&self) -> CacheDir<Compose> {
        CacheDir::at(self.root.join("compose"))
    }
}

/// The directory of one of a project's caches, `K` saying which, so the
/// clip cache cannot be handed to what prunes encoded picture, or the
/// other way round. It reads as a [`Path`] everywhere a path will do.
///
/// ```
/// # use teleprompt_project::{capture::Scenes, project::Project};
/// # fn f(project: &Project, frame: teleprompt_scene::capture::Frame) {
/// let clips = project.caches().clips();
/// Scenes::new(project.registry.scenes, &clips, frame);
/// # }
/// ```
///
/// ```compile_fail
/// # use teleprompt_project::{capture::Scenes, project::Project};
/// # fn f(project: &Project, frame: teleprompt_scene::capture::Frame) {
/// let compose = project.caches().compose();
/// Scenes::new(project.registry.scenes, &compose, frame);
/// # }
/// ```
pub struct CacheDir<K> {
    path: PathBuf,
    kind: PhantomData<K>,
}

/// Synthesized lines: [`CacheDirs::voice`].
pub enum Voice {}
/// Captured clips: [`CacheDirs::clips`].
pub enum Clips {}
/// Encoded chunks of picture: [`CacheDirs::compose`].
pub enum Compose {}

impl<K> CacheDir<K> {
    /// The cache of kind `K` at `path`: one of a project's, or a test's own.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: PhantomData,
        }
    }
}

impl<K> Clone for CacheDir<K> {
    fn clone(&self) -> Self {
        Self::at(self.path.clone())
    }
}

impl<K> std::fmt::Debug for CacheDir<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.path.fmt(f)
    }
}

impl<K> std::ops::Deref for CacheDir<K> {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl<K> AsRef<Path> for CacheDir<K> {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Project {
    /// This project's caches. Every command roots its caches here, so the
    /// cache `check` reads is the one `dub` fills.
    pub fn caches(&self) -> CacheDirs {
        CacheDirs::under(&self.root)
    }

    /// Recorded takes: source, beside the scripts, not a cache.
    pub fn takes_dir(&self) -> PathBuf {
        self.root.join("takes")
    }

    /// Finds the project a `script` belongs to, naming the script on failure.
    pub fn for_script(script: &Path, registry: Registry) -> std::io::Result<Project> {
        Project::discover(script_dir(script), registry).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("cannot locate a project for {}: {e}", script.display()),
            )
        })
    }

    /// Walks up from `from` looking for `teleprompt.toml`; the project found
    /// works with what `registry` has.
    pub fn discover(from: &Path, registry: Registry) -> std::io::Result<Project> {
        let mut dir = from.canonicalize().map_err(|e| {
            std::io::Error::new(e.kind(), format!("cannot read {}: {e}", from.display()))
        })?;
        loop {
            let candidate = dir.join("teleprompt.toml");
            if candidate.exists() {
                let text = std::fs::read_to_string(&candidate)?;
                let mut config = PartialConfig::from_toml(&text).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })?;
                config.root = Some(dir.clone());
                return Ok(Project {
                    root: dir,
                    config,
                    registry,
                });
            }
            if !dir.pop() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no teleprompt.toml found in this directory or any parent",
                ));
            }
        }
    }

    /// The file `config` was read from, which diagnostics about project
    /// settings point at rather than at the script being compiled.
    pub fn config_path(&self) -> PathBuf {
        self.root.join("teleprompt.toml")
    }

    pub fn timeline_path(&self, script: &str, locale: &str) -> PathBuf {
        let stem = Path::new(script)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| script.to_string());
        self.root
            .join("timelines")
            .join(format!("{stem}.{locale}.json"))
    }
}

/// What a watcher compares between looks at a file: its contents, not its
/// mtime. An edit within the filesystem's timestamp resolution leaves the
/// mtime unchanged, and that save is then never seen at all.
pub fn fingerprint(path: &Path) -> Option<teleprompt_core::Hash> {
    std::fs::read(path)
        .ok()
        .map(|bytes| teleprompt_core::Hash::of(&bytes))
}

/// The root of the project around the working directory, or the working
/// directory outside one: where npm packages are looked for.
pub fn root_here(registry: crate::registry::Registry) -> PathBuf {
    let here = PathBuf::from(".");
    Project::discover(&here, registry)
        .map(|p| p.root)
        .unwrap_or(here)
}
