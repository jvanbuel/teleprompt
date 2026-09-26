//! Recorded takes, `takes/<line>.wav`, each with a sidecar recording what
//! was read. They are source, not cache: nothing can make them again.

use std::collections::BTreeMap;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use teleprompt_core::Hash;

use crate::{wav, Pcm};

/// What a take's sidecar holds. Enough to schedule the line without reading
/// its audio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeMeta {
    /// The line as it read when it was recorded.
    pub text: String,
    pub duration_ms: u64,
    /// Of the WAV's bytes.
    pub audio_hash: Hash,
}

/// The takes in one directory, by line id.
#[derive(Debug, Default)]
pub struct Takes {
    dir: PathBuf,
    meta: BTreeMap<String, TakeMeta>,
}

impl Takes {
    /// Reads every sidecar in `dir`; no directory is no takes.
    pub fn load(dir: &Path) -> std::io::Result<Self> {
        let mut meta = BTreeMap::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Ok(Self {
                    dir: dir.to_path_buf(),
                    meta,
                })
            }
            Err(e) => return Err(named(dir, e)),
        };
        for entry in entries {
            let path = entry.map_err(|e| named(dir, e))?.path();
            let Some(id) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".json"))
            else {
                continue;
            };
            let bytes = std::fs::read(&path).map_err(|e| named(&path, e))?;
            let take: TakeMeta = serde_json::from_slice(&bytes)
                .map_err(|e| named(&path, Error::new(ErrorKind::InvalidData, e)))?;
            meta.insert(id.to_string(), take);
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            meta,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.meta.is_empty()
    }

    /// The take of line `id`, if it was read from `text` as it is now.
    pub fn current(&self, id: &str, text: &str) -> Option<&TakeMeta> {
        self.meta.get(id).filter(|t| t.text == text)
    }

    /// Records `pcm` as the take of line `id`, read from `text`, replacing
    /// any earlier one. The audio is written before the sidecar that
    /// vouches for it, each renamed into place.
    pub fn save(&mut self, id: &str, text: &str, pcm: &Pcm) -> std::io::Result<()> {
        if id.is_empty() || id.starts_with('.') || id.contains(['/', '\\']) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("`{id}` cannot name a take"),
            ));
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| named(&self.dir, e))?;
        let bytes = wav::encode(pcm);
        let take = TakeMeta {
            text: text.to_string(),
            duration_ms: pcm.duration_ms(),
            audio_hash: Hash::of(&bytes),
        };
        let sidecar = serde_json::to_vec_pretty(&take).map_err(Error::other)?;
        write_atomic(&self.dir.join(format!("{id}.wav")), &bytes)?;
        write_atomic(&self.dir.join(format!("{id}.json")), &sidecar)?;
        self.meta.insert(id.to_string(), take);
        Ok(())
    }

    /// The WAV of line `id`'s take, refused if it is not the audio its
    /// sidecar describes.
    pub fn read(&self, id: &str) -> std::io::Result<Vec<u8>> {
        let path = self.dir.join(format!("{id}.wav"));
        let Some(take) = self.meta.get(id) else {
            return Err(named(&path, Error::from(ErrorKind::NotFound)));
        };
        let bytes = std::fs::read(&path).map_err(|e| named(&path, e))?;
        if Hash::of(&bytes) != take.audio_hash {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "{} has changed since it was recorded; record the line again",
                    path.display()
                ),
            ));
        }
        Ok(bytes)
    }
}

fn named(path: &Path, e: Error) -> Error {
    Error::new(e.kind(), format!("{}: {e}", path.display()))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("partial");
    std::fs::write(&tmp, bytes).map_err(|e| named(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| named(path, e))
}
