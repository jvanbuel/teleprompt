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
/// A corrupt sidecar is deliberately **not** an error. The cache is
/// content-addressed and entirely derived: every entry can be reproduced
/// from the key that names it, so an unreadable one is missing information,
/// not wrong information. Reporting it as an error made a gitignored,
/// regenerable artifact fail `check` — a validation command — with the blame
/// attributed to the script, and offered the author no way out.
///
/// [`Unusable`](Self::Unusable) is separated from [`Miss`](Self::Miss) only
/// so the caller can say *why* a line it expected to be warm is about to
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

    /// Why a line that should have been warm is about to be re-rendered.
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

/// `Hash` (the trait) is derived so a caller can group entries that share
/// a key — e.g. `dub` grouping narration lines before fan-out, so two
/// lines with identical text render once instead of racing each other
/// to store the same key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
/// release invalidated every cached line and would have failed every
/// downstream consumer's next drift check with "audio changed" on every
/// line. A key turns over when the thing producing the audio changes.
pub fn key(backend_id: &str, backend_version: &str, req: &SynthRequest) -> CacheKey {
    // `-` for no voice, `+v` for `Some(v)`, tagged *before* length-prefixing
    // so `None` and `Some("-")` cannot collapse to the same field.
    let voice = match &req.voice {
        Some(v) => format!("+{v}"),
        None => "-".to_string(),
    };
    CacheKey(Hash::of_fields(&[
        backend_id,
        backend_version,
        &req.locale,
        &voice,
        &req.speed.to_string(),
        &Hash::of(req.text.as_bytes()).to_string(),
    ]))
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

    /// Publish one entry, or adopt the one already published under this key.
    ///
    /// Both files are written to a temporary path in the same directory and
    /// `rename`d into place, so no reader ever sees a partial file and an
    /// interrupted store leaves a temporary behind rather than a truncated
    /// entry. Audio first, sidecar second: `lookup` keys off the sidecar and
    /// [`stats`](Self::stats) counts sidecars, so an interruption between
    /// the two publishes reads as a miss and is not counted, rather than
    /// reading as a hit with no bytes.
    ///
    /// Atomic *per file* is not enough on its own. Two `teleprompt dub`
    /// processes on one project both miss the same key and both store it,
    /// and the interleaving `wav_A → wav_B → sidecar_B → sidecar_A` leaves
    /// a well-formed sidecar beside audio it does not describe — the one
    /// corruption nothing above this layer can detect, because on a cache
    /// hit the duration `dub` publishes and the duration it checks the file
    /// against both come from that same sidecar.
    ///
    /// So the audio's publish is also a *claim*: `hard_link` from the
    /// temporary fails with `AlreadyExists` when something is already at the
    /// path, which makes exactly one writer the publisher of a given key. A
    /// writer that loses the claim waits briefly for the winner's sidecar
    /// and returns the winner's entry — the key is a hash of everything that
    /// changes the audio, so any complete entry under it is *the* answer,
    /// and handing back what is really on disk is what keeps `dub`'s
    /// manifest agreeing with the cache it just read.
    ///
    /// Losing the claim with no usable entry behind it means the path holds
    /// a half-written or corrupt entry from an earlier run, not a writer in
    /// flight; that is healed by replacing both files, which is what keeps a
    /// broken entry from being a permanent miss.
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
    /// `None` means there is nothing to adopt: either the sidecar does not
    /// parse — an entry needing a heal, not a writer in flight, so waiting
    /// would only delay the heal — or nothing appeared before the deadline,
    /// which is what an orphaned WAV from an interrupted run looks like.
    ///
    /// The poll reads metadata, not audio: the wait is for a sidecar to
    /// appear, and re-reading a multi-megabyte WAV every two milliseconds to
    /// discover it has not is the wrong way to ask.
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
        // Now that the pair is complete, read it. If it vanished between the
        // two reads the caller heals it, exactly as for a deadline miss.
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
            // Sidecars, not WAVs, because `lookup` keys off the sidecar: an
            // orphaned `.wav` whose sidecar was lost reads as a miss, so
            // counting it made `doctor` report an entry that does not
            // exist. Its bytes are still counted — they are really on disk,
            // and a reader wondering where the space went is owed that.
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

/// How long a writer that lost the claim waits for the winner's sidecar
/// before deciding there is no winner and healing the entry itself.
///
/// The winner's remaining work is one small write and one `rename`, so the
/// real gap is well under a millisecond and this is a margin, not a budget.
/// It is only ever paid when a WAV is on disk with no sidecar beside it —
/// an interrupted run — because every other outcome is decided by the first
/// poll. Both bounds are deliberately generous in the direction of *not*
/// racing: too long merely delays a heal, whereas too short reintroduces
/// the mismatched pair this claim exists to prevent.
const PUBLISH_WAIT: Duration = Duration::from_millis(250);
const PUBLISH_POLL: Duration = Duration::from_millis(2);

/// Distinguishes the temporary files of two writers. The pid separates
/// processes; this separates calls and threads within one, so no two
/// `store`s can pick the same temporary name and truncate each other's.
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A temporary path beside `path`, never in the system temp directory:
/// `rename` is only atomic within one filesystem and fails outright across
/// two, and the cache directory is the one place guaranteed to be on the
/// same filesystem as the file being published.
fn temp_path(path: &Path) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp.{}.{seq}", std::process::id()));
    path.with_file_name(name)
}

/// Write `bytes` and `rename` them into place, removing the temporary on
/// either failure so a run that dies part way through does not litter a
/// directory `doctor` reports the size of.
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
    /// Nothing was there; this call published the file and owns the key.
    Won,
    /// Something was already there — another writer's file, or the remains
    /// of an interrupted one. Nothing was written.
    Lost,
    /// The filesystem cannot answer the question, so the file was published
    /// the unguarded way. See [`claim`].
    Unchecked,
}

/// Publish `bytes` at `path` only if nothing is there yet.
///
/// `hard_link` is the exclusive-create primitive: it fails with
/// `AlreadyExists` rather than replacing, so of two writers racing on one
/// path exactly one wins, and it publishes the file whole rather than
/// creating it empty and filling it in afterwards.
///
/// A filesystem without hard links (FAT/exFAT removable media) fails it for
/// another reason entirely, and refusing to cache there would be a worse
/// answer than caching without the guard. Those runs fall back to
/// `rename`, which still rules out a torn file and leaves only the
/// two-writer interleaving — exactly where every filesystem stood before.
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
