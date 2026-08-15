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
}

/// What a cache read found.
///
/// A corrupt sidecar is deliberately **not** an error. The cache is
/// content-addressed and entirely derived: every entry can be reproduced
/// from the key that names it, so an unreadable one is missing information,
/// not wrong information. Reporting it as an error made a gitignored,
/// regenerable artifact fail `check` — a validation command — with the blame
/// attributed to the script, and offered the author no way out.
///
/// [`Unusable`](Self::Unusable) is separated from [`Miss`](Self::Miss) only
/// so the caller can say *why* a segment it expected to be warm is about to
/// be synthesized again. Both re-synthesize, and the entry heals.
#[derive(Debug, Clone, PartialEq)]
pub enum CacheRead<T> {
    Hit(T),
    Miss,
    Unusable { path: String, reason: String },
}

impl<T> CacheRead<T> {
    /// The entry, when there was a usable one. A corrupt entry is not one.
    pub fn hit(self) -> Option<T> {
        match self {
            Self::Hit(v) => Some(v),
            _ => None,
        }
    }

    /// Why a segment that should have been warm is about to be re-rendered.
    /// `None` for an ordinary miss, which needs no explanation.
    pub fn warning(&self) -> Option<String> {
        match self {
            Self::Unusable { path, reason } => Some(format!(
                "cache entry {path} could not be read ({reason}); re-synthesizing it"
            )),
            _ => None,
        }
    }
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
    // `-` for no voice, `+v` for `Some(v)`, tagged *before* length-prefixing
    // so `None` and `Some("-")` cannot collapse to the same field.
    let voice = match &req.voice {
        Some(v) => format!("+{v}"),
        None => "-".to_string(),
    };
    let canonical = [
        field(backend_id),
        field(backend_version),
        field(&req.locale),
        field(&voice),
        field(&req.speed.to_string()),
        field(&Hash::of(req.text.as_bytes()).to_string()),
    ]
    .concat();
    CacheKey(Hash::of(canonical.as_bytes()))
}

/// Length-prefixed so the canonical string is injective. Joining
/// user-controlled fields with a separator is not: `locale = "en/US"` with
/// `voice = "af_heart"` and `locale = "en"` with `voice = "US/af_heart"`
/// produce the same string, and a cache collision here silently serves one
/// voice's audio for another.
fn field(s: &str) -> String {
    format!("{}:{}", s.len(), s)
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

/// Duration and word timings without the audio bytes.
///
/// `compile` runs on the inner loop and needs only these; reading the WAV
/// back to discard it would put the whole cache's audio through `plan` on
/// every run.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedMeta {
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub word_timings: Option<Vec<WordTiming>>,
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

    pub fn lookup(&self, key: &CacheKey) -> Result<CacheRead<CachedAudio>, CacheError> {
        let json = self.json_path(key);
        let raw = match std::fs::read_to_string(&json) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(CacheRead::Miss),
            Err(e) => return Err(read_err(&json, e)),
        };
        let side: Sidecar = match serde_json::from_str(&raw) {
            Ok(s) => s,
            Err(e) => return Ok(unusable(&json, &e)),
        };

        // A sidecar with no audio beside it is a half-written entry — from an
        // interrupted `dub`, say. Treat it as absent and let the caller
        // re-synthesize rather than reporting a hit with no bytes.
        let wav_path = self.wav_path(key);
        let wav = match std::fs::read(&wav_path) {
            Ok(w) => w,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(CacheRead::Miss),
            Err(e) => return Err(read_err(&wav_path, e)),
        };

        Ok(CacheRead::Hit(CachedAudio {
            duration_ms: side.duration_ms,
            sample_rate: side.sample_rate,
            channels: side.channels,
            word_timings: side.word_timings,
            wav,
        }))
    }

    /// Like [`lookup`](Self::lookup), but never reads the WAV — only checks
    /// that it exists, so a half-written entry (sidecar with no audio
    /// beside it) still reads as `Ok(None)` rather than a hit with no
    /// bytes.
    pub fn lookup_meta(&self, key: &CacheKey) -> Result<CacheRead<CachedMeta>, CacheError> {
        let json = self.json_path(key);
        let raw = match std::fs::read_to_string(&json) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(CacheRead::Miss),
            Err(e) => return Err(read_err(&json, e)),
        };
        let side: Sidecar = match serde_json::from_str(&raw) {
            Ok(s) => s,
            Err(e) => return Ok(unusable(&json, &e)),
        };

        let wav_path = self.wav_path(key);
        match std::fs::metadata(&wav_path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(CacheRead::Miss),
            Err(e) => return Err(read_err(&wav_path, e)),
        }

        Ok(CacheRead::Hit(CachedMeta {
            duration_ms: side.duration_ms,
            sample_rate: side.sample_rate,
            channels: side.channels,
            word_timings: side.word_timings,
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

fn unusable<T>(path: &Path, source: &serde_json::Error) -> CacheRead<T> {
    CacheRead::Unusable {
        path: path.display().to_string(),
        reason: source.to_string(),
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
