//! `cache`: reporting the voice and compose caches, and keeping `compose/`
//! from growing for ever (docs/design.md#caches).
//!
//! Every entry is derived from its key, so evicting one costs only time.
//! Compose is the one that needs a cap: a minute of video is megabytes,
//! and iterating on a script leaves every version of its picture behind.
//! Least recently used goes first; the renderer marks an entry used when
//! it copies from it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

/// How much encoded video a project keeps when nobody says otherwise.
///
/// Generous: too large costs disk, too small costs re-encoding. A 1080p
/// minute is tens of megabytes, so this holds several versions of a long
/// script.
pub(crate) const DEFAULT_MAX_MB: u64 = 1024;

/// Everything stored under one key.
///
/// Files, plural: a voice entry is a WAV and a sidecar, and evicting one
/// without the other would leave a hit with no bytes behind it.
#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub paths: Vec<PathBuf>,
    pub bytes: u64,
    /// The latest modification time among its files; the renderer touches
    /// an entry it copies from.
    pub used: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Stats {
    pub entries: usize,
    pub bytes: u64,
}

/// What a prune did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Pruned {
    pub removed: usize,
    pub freed: u64,
    /// What is left afterwards.
    pub bytes: u64,
}

/// The entries in a cache directory, oldest use first.
///
/// A directory that does not exist is an empty one: a project that has
/// never built has no cache, and that is not a fault to report.
pub fn entries(dir: &Path) -> Vec<Entry> {
    let Ok(reading) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut by_key: BTreeMap<String, Entry> = BTreeMap::new();
    for file in reading.filter_map(Result::ok) {
        let name = file.file_name().to_string_lossy().into_owned();
        // A dotfile is a half-written entry or a running render's concat
        // list. Neither is data, and neither may be pruned instead.
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = file.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let key = Path::new(&name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(name);
        let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let entry = by_key.entry(key.clone()).or_insert_with(|| Entry {
            key,
            paths: Vec::new(),
            bytes: 0,
            used: SystemTime::UNIX_EPOCH,
        });
        entry.paths.push(file.path());
        entry.bytes += meta.len();
        // The later of the two: a sidecar rewritten on its own still means
        // the key was in use.
        entry.used = entry.used.max(used);
    }

    let mut out: Vec<Entry> = by_key.into_values().collect();
    out.sort_by(|a, b| a.used.cmp(&b.used).then_with(|| a.key.cmp(&b.key)));
    out
}

pub fn stats(dir: &Path) -> Stats {
    let entries = entries(dir);
    Stats {
        entries: entries.len(),
        bytes: entries.iter().map(|e| e.bytes).sum(),
    }
}

/// Evict least-recently-used entries from `dir` until it holds at most
/// `max_bytes`. A cap of zero empties it.
pub fn prune(dir: &Path, max_bytes: u64) -> std::io::Result<Pruned> {
    let entries = entries(dir);
    let mut bytes: u64 = entries.iter().map(|e| e.bytes).sum();
    let mut removed = 0usize;
    let mut freed = 0u64;

    for entry in entries {
        if bytes <= max_bytes {
            break;
        }
        for path in &entry.paths {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                // Another prune or build got there first; not a failure.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        bytes -= entry.bytes;
        freed += entry.bytes;
        removed += 1;
    }

    Ok(Pruned {
        removed,
        freed,
        bytes,
    })
}
