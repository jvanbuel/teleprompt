use std::path::{Path, PathBuf};

use teleprompt_core::config::PartialConfig;

/// A discovered teleprompt project: the directory containing
/// `teleprompt.toml` and that file's parsed contents.
///
/// `Clone` because `serve` hands one to its watcher task, which outlives the
/// call that discovered it. Two fields, both already `Clone`.
#[derive(Clone)]
pub struct Project {
    pub root: PathBuf,
    pub config: PartialConfig,
}

/// The directory a script lives in.
///
/// `Path::new("demo.md").parent()` is `Some("")`, not `None`, so the obvious
/// `script.parent().unwrap_or(Path::new("."))` never falls back — and `""`
/// canonicalizes to ENOENT. Every test, the README, and both manual
/// transcripts happened to pass a path with a directory component, so the
/// bare-filename case (`cd scripts && teleprompt check demo.md`) went unseen.
pub(crate) fn script_dir(script: &Path) -> &Path {
    match script.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

impl Project {
    /// Finds the project a `script` belongs to. Shared by every command that
    /// takes a script path, so the empty-parent handling and the "which file
    /// were we even looking at" context are defined once.
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
        // Naming the path matters: a bare `No such file or directory
        // (os error 2)` tells the author nothing about what was missing.
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

    /// The file `config` was read from. Diagnostics about project settings
    /// point here: a bad `backends:` block is a fact about this file, and
    /// anchoring it to whichever script happened to be compiled sends the
    /// author to the wrong place to fix it.
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
