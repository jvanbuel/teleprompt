use std::path::{Path, PathBuf};

use teleprompt_core::config::PartialConfig;

/// A discovered teleprompt project: the directory containing
/// `teleprompt.toml` and that file's parsed contents.
///
/// `Clone` because the prompter's voice and its threads each keep one.
#[derive(Clone)]
pub struct Project {
    pub root: PathBuf,
    pub config: PartialConfig,
}

/// The directory a script lives in. `Path::new("demo.md").parent()` is
/// `Some("")`, not `None`, and `""` does not canonicalize, so the bare
/// filename case needs `.` spelled out.
pub(crate) fn script_dir(script: &Path) -> &Path {
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
    pub fn voice(&self) -> PathBuf {
        self.root.join("voice")
    }

    /// Captured clips, by capture key.
    pub fn clips(&self) -> PathBuf {
        self.root.join("video")
    }

    /// Encoded chunks of rendered picture.
    pub fn compose(&self) -> PathBuf {
        self.root.join("compose")
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
    pub fn for_script(script: &Path) -> std::io::Result<Project> {
        Project::discover(script_dir(script)).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("cannot locate a project for {}: {e}", script.display()),
            )
        })
    }

    /// Walks up from `from` looking for `teleprompt.toml`.
    pub fn discover(from: &Path) -> std::io::Result<Project> {
        let mut dir = from.canonicalize().map_err(|e| {
            std::io::Error::new(e.kind(), format!("cannot read {}: {e}", from.display()))
        })?;
        loop {
            let candidate = dir.join("teleprompt.toml");
            if candidate.exists() {
                let text = std::fs::read_to_string(&candidate)?;
                let config = PartialConfig::from_toml(&text).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })?;
                return Ok(Project { root: dir, config });
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
    pub(crate) fn config_path(&self) -> PathBuf {
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
