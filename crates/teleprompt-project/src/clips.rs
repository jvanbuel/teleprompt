//! The clip cache: each captured shot's picture, by capture key, under the
//! name a renderer looks for ([`clip_file`]). A clip is here only once
//! the session that recorded it has finished, so whatever is here is taken
//! as captured.

use std::path::{Path, PathBuf};

use teleprompt_core::Hash;
use teleprompt_manifest::{clip_file, NarrationManifest};
use teleprompt_scene::capture::{Clip, WorkDir};

/// A project's captured clips.
#[derive(Debug, Clone)]
pub struct ClipStore {
    dir: PathBuf,
}

impl ClipStore {
    /// The clips kept in `dir`: a project's, or a test's own.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Where they are, for a renderer that reads them by [`clip_file`],
    /// and for reporting.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn has(&self, key: &Hash) -> bool {
        self.dir.join(clip_file(key)).is_file()
    }

    /// The clip captured under `key`, if there is one.
    pub fn path_of(&self, key: &Hash) -> Option<PathBuf> {
        Some(self.dir.join(clip_file(key))).filter(|p| p.is_file())
    }

    /// Files the clips `record` makes in the staging directory it is
    /// given, each under its key, returning how many; or, if `record`
    /// fails, files none of them and says why.
    ///
    /// The staging directory is beside the clips, so filing is a rename,
    /// and is dropped with whatever a failed run left in it.
    pub fn record(
        &self,
        record: impl FnOnce(&Path) -> Result<Vec<Clip>, String>,
    ) -> Result<usize, String> {
        let staging = std::fs::create_dir_all(&self.dir)
            .and_then(|()| WorkDir::create(&self.dir, "staging"))
            .map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let clips = record(&staging)?;
        for clip in &clips {
            let into = self.dir.join(clip_file(&clip.key));
            std::fs::rename(&clip.path, &into)
                .map_err(|e| format!("cannot file {}: {e}", into.display()))?;
        }
        Ok(clips.len())
    }

    /// Every clip `manifest` names that is here, in `into`, and nothing
    /// else there. Hard links where the file system allows, so a package
    /// costs no space; copies, renamed into place, where it does not.
    pub fn place_into(
        &self,
        manifest: &NarrationManifest,
        into: &Path,
    ) -> std::io::Result<Vec<PathBuf>> {
        std::fs::create_dir_all(into)?;
        let named: std::collections::BTreeSet<String> = manifest
            .shots
            .iter()
            .filter_map(|s| s.clip_name())
            .collect();
        for entry in std::fs::read_dir(into)? {
            let entry = entry?;
            if !named.contains(&*entry.file_name().to_string_lossy()) {
                std::fs::remove_file(entry.path())?;
            }
        }
        let mut placed = Vec::new();
        for name in &named {
            let (from, to) = (self.dir.join(name), into.join(name));
            if !from.is_file() {
                continue;
            }
            if to.exists() {
                std::fs::remove_file(&to)?;
            }
            if std::fs::hard_link(&from, &to).is_err() {
                let partial = into.join(format!("{name}.partial"));
                std::fs::copy(&from, &partial)?;
                std::fs::rename(&partial, &to)?;
            }
            placed.push(to);
        }
        Ok(placed)
    }
}
