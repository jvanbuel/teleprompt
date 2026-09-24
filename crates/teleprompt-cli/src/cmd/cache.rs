//! The caches a project keeps, and how they are kept from growing for ever.
//!
//! Two of them sit under `.teleprompt/cache`: `voice/`, which holds
//! synthesized narration, and `compose/`, which holds already-encoded
//! pieces of picture. Both are entirely derived — every entry can be
//! reproduced from the key that names it — so losing one costs time and
//! nothing else. That is what makes evicting from them safe.
//!
//! It is `compose/` that needs it. A sentence of narration is kilobytes; a
//! minute of video is megabytes, and a script iterated on through a
//! morning leaves every intermediate version of its picture behind. The
//! policy is the plainest one that keeps a cache worth having: a cap, and
//! the least recently used entry goes first. The renderer marks an entry
//! used when it copies from it, so what the last few builds depended on is
//! the last thing to be thrown away.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

use crate::cmd::check::cache_root;
use crate::project::Project;

/// How much encoded video a project keeps when nobody says otherwise.
///
/// Generous: the cost of it being too large is disk, and the cost of it
/// being too small is an author paying for a render they had already paid
/// for. A 1080p minute is some tens of megabytes, so this is room for a
/// long script and several versions of it.
pub(crate) const DEFAULT_MAX_MB: u64 = 1024;

/// Everything stored under one key.
///
/// Files, plural: the voice cache writes a WAV and a sidecar per key, and
/// evicting the audio while keeping the sidecar would leave a hit with no
/// bytes behind it.
#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub paths: Vec<PathBuf>,
    pub bytes: u64,
    /// When this entry was last used, which is its modification time: the
    /// renderer touches an entry it copies from.
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
        // A dotfile is a half-written entry — the renderer encodes beside
        // the name it is going to use and renames into place — or the
        // concat list of a render still running. Neither is data, and
        // neither may be what a prune decides to throw away instead.
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
/// `max_bytes`.
///
/// A cap of zero keeps nothing, which is how an author gets the disk back.
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
                // Somebody else's prune got there first, or a build in
                // another terminal just rewrote it. Neither is this
                // command's business to fail over.
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

/// Where each cache lives for a project.
pub fn voice_dir(project: &Project) -> PathBuf {
    cache_root(project).join("voice")
}

pub fn compose_dir(project: &Project) -> PathBuf {
    cache_root(project).join("compose")
}

#[derive(Debug, Serialize)]
pub struct CacheReport {
    pub ok: bool,
    pub root: PathBuf,
    pub voice: Stats,
    pub compose: Stats,
    /// What a `prune` did, or `null` for a report that only looked.
    pub pruned: Option<Pruned>,
}

impl CacheReport {
    pub fn render(&self) -> String {
        let mut out = format!("  {}\n", self.root.display());
        out.push_str(&format!(
            "  voice            {} entries, {}\n",
            self.voice.entries,
            size(self.voice.bytes)
        ));
        out.push_str(&format!(
            "  compose          {} entries, {}\n",
            self.compose.entries,
            size(self.compose.bytes)
        ));
        if let Some(pruned) = self.pruned {
            out.push_str(&format!(
                "  pruned           {} entries, {} freed\n",
                pruned.removed,
                size(pruned.freed)
            ));
        }
        out
    }
}

/// Bytes, in the unit a human would have said them in.
fn size(bytes: u64) -> String {
    const MB: f64 = 1_048_576.0;
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.0} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / MB),
    }
}

/// Report the caches, having first pruned the compose cache to `max_mb` if
/// one was asked for.
pub fn run_cache(project: &Project, max_mb: Option<u64>) -> std::io::Result<CacheReport> {
    let pruned = match max_mb {
        Some(mb) => Some(prune(&compose_dir(project), mb * 1_048_576)?),
        None => None,
    };
    Ok(CacheReport {
        ok: true,
        root: cache_root(project),
        voice: stats(&voice_dir(project)),
        compose: stats(&compose_dir(project)),
        pruned,
    })
}
