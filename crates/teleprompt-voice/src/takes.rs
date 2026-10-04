//! Recorded takes, `takes/<line>.wav`, each with a sidecar recording what
//! was read. They are source, not cache: nothing can make them again.

use std::collections::BTreeMap;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use teleprompt_core::Hash;

use teleprompt_plugin::voice::{wav, Pcm};

/// What a take's sidecar holds. Enough to schedule the line without reading
/// its audio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeMeta {
    /// The line as it read when it was recorded.
    pub text: String,
    pub duration_ms: u64,
    /// Of the WAV's bytes.
    pub audio_hash: Hash,
    /// What the recognizer heard, where it listened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heard: Option<String>,
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

    /// Every take, by line id, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &TakeMeta)> {
        self.meta.iter().map(|(id, t)| (id.as_str(), t))
    }

    pub fn is_empty(&self) -> bool {
        self.meta.is_empty()
    }

    /// The take of line `id`, if it was read from `text` as it is now.
    pub fn current(&self, id: &str, text: &str) -> Option<&TakeMeta> {
        self.meta.get(id).filter(|t| t.text == text)
    }

    /// Whether line `id` has a take of other words than `text`: reworded
    /// since it was recorded, so due to be recorded again.
    pub fn stale(&self, id: &str, text: &str) -> bool {
        self.meta.get(id).is_some_and(|t| t.text != text)
    }

    /// Records `pcm` as the take of line `id`, read from `text`, replacing
    /// any earlier one. The audio is written before the sidecar that
    /// vouches for it, each renamed into place.
    pub fn save(&mut self, id: &str, text: &str, pcm: &Pcm) -> std::io::Result<()> {
        self.save_take(id, text, None, pcm)
    }

    /// As [`save`](Self::save), with what the recognizer heard said.
    pub fn save_heard(
        &mut self,
        id: &str,
        text: &str,
        heard: &str,
        pcm: &Pcm,
    ) -> std::io::Result<()> {
        self.save_take(id, text, Some(heard), pcm)
    }

    /// Line `id` as its current take was heard to say it, where that is
    /// other words than `text` (`teleprompt_core::said`).
    pub fn said(&self, id: &str, text: &str) -> Option<String> {
        let heard = self.current(id, text)?.heard.as_deref()?;
        teleprompt_core::said::reworded(text, heard)
    }

    /// Line `id`'s take, now of the words `text`: the line was reworded to
    /// what the take says.
    pub fn retext(&mut self, id: &str, text: &str) -> std::io::Result<()> {
        let path = self.dir.join(format!("{id}.json"));
        let Some(take) = self.meta.get_mut(id) else {
            return Err(named(&path, Error::from(ErrorKind::NotFound)));
        };
        take.text = text.to_string();
        let sidecar = serde_json::to_vec_pretty(take).map_err(Error::other)?;
        write_atomic(&path, &sidecar)
    }

    fn save_take(
        &mut self,
        id: &str,
        text: &str,
        heard: Option<&str>,
        pcm: &Pcm,
    ) -> std::io::Result<()> {
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
            heard: heard.map(str::to_string),
        };
        let sidecar = serde_json::to_vec_pretty(&take).map_err(Error::other)?;
        self.put_aside(id)?;
        write_atomic(&self.dir.join(format!("{id}.wav")), &bytes)?;
        write_atomic(&self.dir.join(format!("{id}.json")), &sidecar)?;
        self.meta.insert(id.to_string(), take);
        Ok(())
    }

    /// Where the take a save replaced waits, for [`restore`](Self::restore):
    /// hidden, so loading the directory does not list it.
    fn aside(&self) -> PathBuf {
        self.dir.join(".previous")
    }

    /// Moves line `id`'s take aside, or marks that it had none.
    fn put_aside(&self, id: &str) -> std::io::Result<()> {
        let aside = self.aside();
        std::fs::create_dir_all(&aside).map_err(|e| named(&aside, e))?;
        let none = aside.join(format!("{id}.none"));
        if self.meta.contains_key(id) {
            for ext in ["wav", "json"] {
                let from = self.dir.join(format!("{id}.{ext}"));
                std::fs::rename(&from, aside.join(format!("{id}.{ext}")))
                    .map_err(|e| named(&from, e))?;
            }
            let _ = std::fs::remove_file(&none);
        } else {
            for ext in ["wav", "json"] {
                let _ = std::fs::remove_file(aside.join(format!("{id}.{ext}")));
            }
            std::fs::write(&none, b"").map_err(|e| named(&none, e))?;
        }
        Ok(())
    }

    /// Puts back what line `id`'s last save replaced: the take before it,
    /// or no take. Whether there was anything to put back; once only.
    pub fn restore(&mut self, id: &str) -> std::io::Result<bool> {
        let aside = self.aside();
        let none = aside.join(format!("{id}.none"));
        let sidecar = aside.join(format!("{id}.json"));
        if none.exists() {
            for ext in ["wav", "json"] {
                let path = self.dir.join(format!("{id}.{ext}"));
                match std::fs::remove_file(&path) {
                    Err(e) if e.kind() != ErrorKind::NotFound => return Err(named(&path, e)),
                    _ => {}
                }
            }
            self.meta.remove(id);
            std::fs::remove_file(&none).map_err(|e| named(&none, e))?;
            return Ok(true);
        }
        if !sidecar.exists() {
            return Ok(false);
        }
        let bytes = std::fs::read(&sidecar).map_err(|e| named(&sidecar, e))?;
        let take: TakeMeta = serde_json::from_slice(&bytes)
            .map_err(|e| named(&sidecar, Error::new(ErrorKind::InvalidData, e)))?;
        // The audio first, then the sidecar that vouches for it.
        for ext in ["wav", "json"] {
            let from = aside.join(format!("{id}.{ext}"));
            std::fs::rename(&from, self.dir.join(format!("{id}.{ext}")))
                .map_err(|e| named(&from, e))?;
        }
        self.meta.insert(id.to_string(), take);
        Ok(true)
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
