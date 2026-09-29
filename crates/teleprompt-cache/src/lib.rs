//! Content-addressed cache for synthesized narration
//! (`docs/design.md#voice-cache`). `plan` and `diff` read durations from it;
//! only `dub` and `build` write to it. It knows nothing about backends.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

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
/// A corrupt sidecar is not an error: every entry is derived, so an
/// unreadable one is missing, not wrong, and failing `check` over it would
/// blame the script. [`Unusable`](Self::Unusable) differs from
/// [`Miss`](Self::Miss) only so the caller can say why a line it expected
/// warm is synthesized again; both re-synthesize, and the entry heals.
#[derive(Debug, Clone, PartialEq)]
pub enum CacheRead<T> {
    Hit(T),
    Miss,
    Unusable { path: String, reason: String },
}

impl<T> CacheRead<T> {
    /// The entry, when there was a usable one.
    pub fn hit(self) -> Option<T> {
        match self {
            Self::Hit(v) => Some(v),
            _ => None,
        }
    }

    /// Why a line expected warm is re-synthesized; `None` for a plain miss.
    pub fn warning(&self) -> Option<String> {
        match self {
            Self::Unusable { path, reason } => Some(format!(
                "cache entry {path} could not be read ({reason}); re-synthesizing it"
            )),
            _ => None,
        }
    }
}

/// Derives `Hash` so `dub` can group lines that share a key and synthesize
/// each key once, rather than racing two stores of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey(Hash);

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Everything that changes the audio, and nothing that does not
/// (`docs/design.md#voice-cache`). `backend_version` is the backend's
/// version, never teleprompt's.
pub fn key(backend_id: &str, backend_version: &str, req: &SynthRequest) -> CacheKey {
    // Tagged before length-prefixing, so `None` and `Some("-")` differ.
    let voice = match &req.voice {
        Some(v) => format!("+{v}"),
        None => "-".to_string(),
    };
    let speed = req.speed.to_string();
    let text = Hash::of(req.text.as_bytes()).to_string();
    let mut fields = vec![
        backend_id,
        backend_version,
        &req.locale,
        &voice,
        &speed,
        &text,
    ];
    // Only when given, so a request without instructions keys as it did
    // before they existed and no cache is thrown away.
    let instruct = req.instruct.as_ref().map(|i| format!("instruct:{i}"));
    fields.extend(instruct.as_deref());
    CacheKey(Hash::of_fields(&fields))
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

/// Duration and word timings without the audio bytes, so the inner loop
/// never reads the cache's audio.
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

        // A sidecar with no audio beside it is a half-written entry: a miss,
        // not a hit with no bytes.
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

    /// Like [`lookup`](Self::lookup), but only checks that the WAV exists,
    /// so a half-written entry still reads as [`CacheRead::Miss`].
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

    /// Publish one entry, or adopt the one already published under this key.
    ///
    /// Each file is written to a temporary beside it and renamed into place,
    /// audio first: `lookup` and [`stats`](Self::stats) key off the sidecar,
    /// so an interrupted store reads as a miss, never as a hit with no bytes.
    ///
    /// Per-file atomicity is not enough: two `dub` processes storing one key
    /// can interleave into a valid sidecar beside audio it does not describe,
    /// which nothing above this layer can detect. So publishing the audio is
    /// also an exclusive claim (`claim`): one writer wins, and a loser adopts
    /// the winner's entry once its sidecar lands. A lost claim with nothing
    /// usable behind it is an interrupted run's, healed by rewriting both.
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

        let wav_path = self.wav_path(key);
        match claim(&wav_path, &entry.wav)? {
            Claim::Won | Claim::Unchecked => {}
            Claim::Lost => match self.await_published(key)? {
                Some(published) => return Ok(published),
                None => write_atomic(&wav_path, &entry.wav)?,
            },
        }

        let side = Sidecar {
            duration_ms: entry.duration_ms,
            sample_rate: entry.sample_rate,
            channels: entry.channels,
            word_timings: entry.word_timings.clone(),
        };
        let json_path = self.json_path(key);
        let json = serde_json::to_string(&side).expect("Sidecar always serializes");
        write_atomic(&json_path, json.as_bytes())?;

        Ok(entry)
    }

    /// The entry another writer is publishing under `key`, once it lands.
    ///
    /// `None` means nothing to adopt: a corrupt sidecar (heal now rather than
    /// wait) or none by the deadline (an orphaned WAV). Polls metadata only.
    fn await_published(&self, key: &CacheKey) -> Result<Option<CachedAudio>, CacheError> {
        let deadline = Instant::now() + PUBLISH_WAIT;
        loop {
            match self.lookup_meta(key)? {
                CacheRead::Hit(_) => break,
                CacheRead::Unusable { .. } => return Ok(None),
                CacheRead::Miss => {}
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(PUBLISH_POLL);
        }
        // If the entry vanished between the two reads, the caller heals it.
        Ok(self.lookup(key)?.hit())
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
            // Count sidecars, as `lookup` keys off them, so an orphaned WAV
            // is not an entry; its bytes still count, being on disk.
            if path.extension().is_some_and(|e| e == "json") {
                count += 1;
            }
        }
        Ok(CacheStats {
            entries: count,
            bytes,
        })
    }
}

/// How long a writer that lost the [`claim`] waits for the winner's sidecar
/// before healing the entry itself. The winner has one small write and a
/// `rename` left, so this is a margin, paid only for an orphaned WAV. Too
/// long merely delays a heal; too short lets two writers race again.
const PUBLISH_WAIT: Duration = Duration::from_millis(250);
const PUBLISH_POLL: Duration = Duration::from_millis(2);

/// With the pid, keeps temporary names unique across processes, threads and
/// calls, so no two stores truncate each other's temporary.
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Beside `path`, never in the system temp directory: `rename` is atomic
/// only within one filesystem.
fn temp_path(path: &Path) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp.{}.{seq}", std::process::id()));
    path.with_file_name(name)
}

/// Write to a temporary and `rename` it into place, removing the temporary
/// on failure so it does not litter the cache.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CacheError> {
    let tmp = temp_path(path);
    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(write_err(&tmp, e));
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(write_err(path, e));
    }
    Ok(())
}

/// What [`claim`] found at the path.
enum Claim {
    /// Nothing was there; this call published the file.
    Won,
    /// Something was there already. Nothing was written.
    Lost,
    /// No hard links here, so the file was published unguarded.
    Unchecked,
}

/// Publish `bytes` at `path` only if nothing is there yet.
///
/// `hard_link` fails with `AlreadyExists` rather than replacing, so of two
/// racing writers exactly one wins, and the file appears whole. Filesystems
/// without hard links (FAT/exFAT) fall back to `rename`: no torn file, only
/// the two-writer race unguarded, which beats not caching there at all.
fn claim(path: &Path, bytes: &[u8]) -> Result<Claim, CacheError> {
    let tmp = temp_path(path);
    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(write_err(&tmp, e));
    }
    match std::fs::hard_link(&tmp, path) {
        Ok(()) => {
            let _ = std::fs::remove_file(&tmp);
            Ok(Claim::Won)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&tmp);
            Ok(Claim::Lost)
        }
        Err(_) => {
            if let Err(e) = std::fs::rename(&tmp, path) {
                let _ = std::fs::remove_file(&tmp);
                return Err(write_err(path, e));
            }
            Ok(Claim::Unchecked)
        }
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
