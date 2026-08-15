use std::path::{Path, PathBuf};

use teleprompt_core::config::PartialConfig;

/// A discovered teleprompt project: the directory containing
/// `teleprompt.toml` and that file's parsed contents.
pub struct Project {
    pub root: PathBuf,
    pub config: PartialConfig,
}

impl Project {
    /// Walks up from `from` looking for `teleprompt.toml`.
    pub fn discover(from: &Path) -> std::io::Result<Project> {
        let mut dir = from.canonicalize()?;
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
