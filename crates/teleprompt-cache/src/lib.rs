//! Content-addressed cache for synthesized narration.
//!
//! This is what makes a slow backend usable: `plan` and `diff` read
//! durations from here rather than running a model, and only `dub` and
//! `build` ever populate it.
//!
//! The cache knows nothing about backends. It is given a key and some
//! samples, and it stores them.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use teleprompt_core::Hash;
use teleprompt_voice::{wav, Pcm, SynthRequest, WordTiming};

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("cannot read cache entry {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write cache entry {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("cache entry {path} is corrupt: {source}")]
    Corrupt {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey(Hash);

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Everything that changes the audio, and nothing that does not.
///
/// `backend_version` is the *backend's* version, never teleprompt's. An
/// earlier design keyed on `CARGO_PKG_VERSION`, which meant every teleprompt
/// release invalidated every cached segment and would have failed every
/// downstream consumer's next drift check with "audio changed" on every
/// segment. A key turns over when the thing producing the audio changes.
pub fn key(backend_id: &str, backend_version: &str, req: &SynthRequest) -> CacheKey {
    let canonical = format!(
        "{backend_id}/{backend_version}/{}/{}/{}/{}",
        req.locale,
        req.voice.as_deref().unwrap_or("-"),
        req.speed,
        Hash::of(req.text.as_bytes()),
    );
    CacheKey(Hash::of(canonical.as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Sidecar {
    duration_ms: u64,
    sample_rate: u32,
    channels: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    word_timings: Option<Vec<WordTiming>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CachedAudio {
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub word_timings: Option<Vec<WordTiming>>,
    pub wav: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: u64,
}

pub struct VoiceCache {
    root: PathBuf,
}

impl VoiceCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn dir(&self) -> PathBuf {
        self.root.join("voice")
    }

    fn wav_path(&self, key: &CacheKey) -> PathBuf {
        self.dir().join(format!("{key}.wav"))
    }

    fn json_path(&self, key: &CacheKey) -> PathBuf {
        self.dir().join(format!("{key}.json"))
    }

    pub fn lookup(&self, key: &CacheKey) -> Result<Option<CachedAudio>, CacheError> {
        let json = self.json_path(key);
        let raw = match std::fs::read_to_string(&json) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(read_err(&json, e)),
        };
        let side: Sidecar = serde_json::from_str(&raw).map_err(|source| CacheError::Corrupt {
            path: json.display().to_string(),
            source,
        })?;

        // A sidecar with no audio beside it is a half-written entry — from an
        // interrupted `dub`, say. Treat it as absent and let the caller
        // re-synthesize rather than reporting a hit with no bytes.
        let wav_path = self.wav_path(key);
        let wav = match std::fs::read(&wav_path) {
            Ok(w) => w,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(read_err(&wav_path, e)),
        };

        Ok(Some(CachedAudio {
            duration_ms: side.duration_ms,
            sample_rate: side.sample_rate,
            channels: side.channels,
            word_timings: side.word_timings,
            wav,
        }))
    }

    pub fn store(
        &self,
        key: &CacheKey,
        pcm: &Pcm,
        word_timings: Option<&[WordTiming]>,
    ) -> Result<CachedAudio, CacheError> {
        let dir = self.dir();
        std::fs::create_dir_all(&dir).map_err(|e| write_err(&dir, e))?;

        let entry = CachedAudio {
            duration_ms: pcm.duration_ms(),
            sample_rate: pcm.sample_rate,
            channels: pcm.channels,
            word_timings: word_timings.map(<[WordTiming]>::to_vec),
            wav: wav::encode(pcm),
        };

        // Audio first, sidecar second: `lookup` keys off the sidecar, so an
        // interruption between the two writes leaves an entry that reads as
        // a miss rather than as a hit with no bytes.
        let wav_path = self.wav_path(key);
        std::fs::write(&wav_path, &entry.wav).map_err(|e| write_err(&wav_path, e))?;

        let side = Sidecar {
            duration_ms: entry.duration_ms,
            sample_rate: entry.sample_rate,
            channels: entry.channels,
            word_timings: entry.word_timings.clone(),
        };
        let json_path = self.json_path(key);
        let json = serde_json::to_string(&side).expect("Sidecar always serializes");
        std::fs::write(&json_path, json).map_err(|e| write_err(&json_path, e))?;

        Ok(entry)
    }

    pub fn stats(&self) -> Result<CacheStats, CacheError> {
        let dir = self.dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CacheStats {
                    entries: 0,
                    bytes: 0,
                })
            }
            Err(e) => return Err(read_err(&dir, e)),
        };

        let mut count = 0;
        let mut bytes = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            bytes += meta.len();
            if path.extension().is_some_and(|e| e == "wav") {
                count += 1;
            }
        }
        Ok(CacheStats {
            entries: count,
            bytes,
        })
    }
}

fn read_err(path: &Path, source: std::io::Error) -> CacheError {
    CacheError::Read {
        path: path.display().to_string(),
        source,
    }
}

fn write_err(path: &Path, source: std::io::Error) -> CacheError {
    CacheError::Write {
        path: path.display().to_string(),
        source,
    }
}
