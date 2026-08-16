# Pluggable Voice Backends — Delivery A Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the voice backend a sharp, one-method, pluggable contract, and get the backend off the inner loop so a slow real synthesizer can exist without making `plan` slow.

**Architecture:** Backends move into their own crates behind a registry, with `null` moved out first so the contract is proven from outside. A content-addressed cache and a synchronous `DurationEstimator` take over the hot path, so `compile()` stops calling backends entirely — which is what lets the trait become async without infecting `check`, `plan`, or `diff`.

**Tech Stack:** Rust 2021, `async-trait`, `tokio` (CLI only), `blake3`, `serde`/`serde_json`, `insta`.

**Spec:** `docs/superpowers/specs/2026-08-15-teleprompt-voice-backends-design.md`. It depends on `docs/superpowers/specs/2026-08-15-teleprompt-design.md`; unqualified `§` references point at the latter, matching the spec's own convention.

**Scope:** This is **Delivery A** of the spec's §14 split. It ships the contract, the crate layout, the registry, the cache, and `duration_source`. It ships **no Kokoro** — Delivery B is a separate plan. Delivery A is worth shipping alone: it makes the timeline honest about which durations are measured and which are predicted.

## Global Constraints

- MSRV is **1.75**. CI runs `cargo check` on 1.75.
- New workspace dependencies are permitted **only** for `async-trait` and `tokio` (features `rt-multi-thread`, `macros`). Anything else is out of scope for Delivery A — `reqwest` belongs to Delivery B.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` must pass at every commit.
- **The workspace must compile and the whole suite must pass at every commit.** Task order is chosen to make that possible; do not reorder tasks.
- Output must be deterministic and byte-stable: no timestamps, no `HashMap` iteration, no floats in serialized output.
- Exit codes are fixed by §10.1 and issued only via `teleprompt_cli::output::exit_code_for`.
- **`check`, `plan`, and `diff` must never call a `VoiceBackend` and must never start an async runtime.** This is the spec's central property; a task that violates it has failed regardless of its tests.
- The test suite gains no network dependency, no model download, and no Node.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/teleprompt-voice/src/contract.rs` (modify) | The trait, `SynthRequest`, `Synthesized`, `Pcm`, `WordTiming`, `VoiceCapabilities`, `VoiceError`. |
| `crates/teleprompt-voice/src/estimator.rs` (create) | `DurationEstimator` — the trait only. |
| `crates/teleprompt-voice/src/registry.rs` (create) | `VoiceRegistry`, mirroring `SceneRegistry`. |
| `crates/teleprompt-voice-null/` (create) | `NullVoice` and `WpmEstimator`, built against `teleprompt-voice`'s public API only. |
| `crates/teleprompt-cache/` (create) | `VoiceCache`: content-addressed lookup and store. Knows nothing about backends. |
| `crates/teleprompt-schedule/src/beat.rs` (modify) | `NarrationInput.duration_source`. |
| `crates/teleprompt-schedule/src/timeline.rs` (modify) | `NarrationEntry.duration_source`. |
| `crates/teleprompt-schedule/src/diff.rs` (modify) | The `now measured` reason. |
| `crates/teleprompt-compile/src/lib.rs` (modify) | Takes a cache and an estimator; no longer takes a backend. |
| `crates/teleprompt-cli/src/cmd/dub.rs` (modify) | The only async entry point; synthesizes and fills the cache. |
| `crates/teleprompt-cli/src/voice.rs` (create) | Builds the registry and resolves `VoiceConfig.backend` against it. |

---

### Task 1: `teleprompt-voice-null` — move the reference implementation out

The structural check on the whole design. `NullVoice` must be writable from outside `teleprompt-voice` using only its public API. **If it needs something the contract crate does not export, that is a finding — report it rather than adding a `pub(crate)` escape hatch.**

The contract itself does not change in this task, so the workspace stays green.

**Files:**
- Create: `crates/teleprompt-voice-null/Cargo.toml`, `crates/teleprompt-voice-null/src/lib.rs`, `crates/teleprompt-voice-null/src/estimator.rs`
- Create: `crates/teleprompt-voice/src/estimator.rs`
- Modify: `crates/teleprompt-voice/src/lib.rs` (drop `pub mod null`, add `pub mod estimator`)
- Delete: `crates/teleprompt-voice/src/null.rs`
- Move: `crates/teleprompt-voice/tests/null.rs` → `crates/teleprompt-voice-null/tests/null.rs`
- Modify: `crates/teleprompt-compile/src/lib.rs`, `crates/teleprompt-compile/tests/*.rs`, `crates/teleprompt-cli/src/cmd/check.rs`, `crates/teleprompt-cli/src/cmd/dub.rs`, and their `Cargo.toml`s — import path only

**Interfaces:**
- Produces:
  - `teleprompt_voice::estimator::DurationEstimator` with `fn estimate_ms(&self, req: &SynthRequest) -> u64`
  - `teleprompt_voice_null::NullVoice`, `teleprompt_voice_null::WpmEstimator`, `teleprompt_voice_null::NULL_SAMPLE_RATE`
  - `WpmEstimator { pub wpm: f64 }` with `Default` giving `wpm: 150.0`

- [ ] **Step 1: Write the failing test**

Create `crates/teleprompt-voice-null/tests/estimator.rs`:

```rust
use teleprompt_voice::estimator::DurationEstimator;
use teleprompt_voice::SynthRequest;
use teleprompt_voice_null::WpmEstimator;

fn req(text: &str, speed: f64) -> SynthRequest {
    SynthRequest {
        text: text.to_string(),
        locale: "en".to_string(),
        voice: None,
        speed,
    }
}

#[test]
fn six_words_at_150_wpm_is_2400ms() {
    let e = WpmEstimator::default();
    assert_eq!(e.estimate_ms(&req("one two three four five six", 1.0)), 2400);
}

#[test]
fn punctuation_adds_pauses() {
    let e = WpmEstimator::default();
    let plain = e.estimate_ms(&req("one two three four five six", 1.0));
    let punctuated = e.estimate_ms(&req("one two three, four five six.", 1.0));
    assert_eq!(punctuated, plain + 150 + 350, "comma 150ms, sentence end 350ms");
}

#[test]
fn speed_divides_the_duration() {
    let e = WpmEstimator::default();
    let normal = e.estimate_ms(&req("one two three four five six", 1.0));
    assert_eq!(e.estimate_ms(&req("one two three four five six", 2.0)), normal / 2);
}

#[test]
fn estimation_is_deterministic() {
    let e = WpmEstimator::default();
    let r = req("Every video here is built from a script.", 1.0);
    assert_eq!(e.estimate_ms(&r), e.estimate_ms(&r));
}

/// The structural claim of the whole design: the reference implementation is
/// written from outside the contract crate, touching only its public API. If
/// this file ever needs a `pub(crate)` item, the contract is wrong.
#[test]
fn the_estimator_is_reachable_through_the_trait_object() {
    let e: Box<dyn DurationEstimator> = Box::new(WpmEstimator::default());
    assert!(e.estimate_ms(&req("hello there", 1.0)) > 0);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p teleprompt-voice-null`
Expected: FAIL — the package does not exist.

- [ ] **Step 3: Add the estimator trait to the contract crate**

Create `crates/teleprompt-voice/src/estimator.rs`:

```rust
use crate::contract::SynthRequest;

/// Predicts how long text takes to speak *without synthesizing it*.
///
/// Separate from [`crate::VoiceBackend`] on purpose. A backend runs a model
/// and takes seconds; the inner loop (`plan`, `diff`) needs a duration for
/// every segment on every keystroke-driven run. Keeping prediction out of
/// the backend contract is what lets the backend be async while this stays
/// synchronous and free.
///
/// Implementations must be deterministic: the same request always yields the
/// same number, or committed timelines churn.
pub trait DurationEstimator: Send + Sync {
    fn estimate_ms(&self, req: &SynthRequest) -> u64;
}
```

In `crates/teleprompt-voice/src/lib.rs`, replace `pub mod null;` with `pub mod estimator;`, drop the `pub use null::{...}` line, and add `pub use estimator::DurationEstimator;`.

- [ ] **Step 4: Create the crate and move the implementation**

`crates/teleprompt-voice-null/Cargo.toml`:

```toml
[package]
name = "teleprompt-voice-null"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
teleprompt-core = { path = "../teleprompt-core" }
teleprompt-voice = { path = "../teleprompt-voice" }
```

`crates/teleprompt-voice-null/src/lib.rs`:

```rust
//! The reference voice backend: silence of exactly the estimated length,
//! and the word-count duration model behind it.
//!
//! This lives outside `teleprompt-voice` deliberately. If the reference
//! implementation cannot be written from outside the contract crate using
//! only its public API, the contract is not a contract.

pub mod estimator;
mod null;

pub use estimator::WpmEstimator;
pub use null::{NullVoice, NULL_SAMPLE_RATE};
```

Move the body of the old `crates/teleprompt-voice/src/null.rs` to `crates/teleprompt-voice-null/src/null.rs`, changing `use crate::contract::{...}` to `use teleprompt_voice::{...}`. Move `estimate_ms` and the three pause constants out into `crates/teleprompt-voice-null/src/estimator.rs`:

```rust
use teleprompt_voice::estimator::DurationEstimator;
use teleprompt_voice::SynthRequest;

const COMMA_MS: u64 = 150;
const CLAUSE_MS: u64 = 250;
const SENTENCE_MS: u64 = 350;

/// Words per minute plus punctuation pauses. Crude, deterministic, and free
/// — which is what the inner loop needs.
pub struct WpmEstimator {
    pub wpm: f64,
}

impl Default for WpmEstimator {
    fn default() -> Self {
        Self { wpm: 150.0 }
    }
}

/// Free function so `NullVoice` can share it without owning an estimator.
pub fn estimate_ms(text: &str, wpm: f64, speed: f64) -> u64 {
    let words = text
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count() as f64;
    if words == 0.0 {
        return 0;
    }
    let speech = words / wpm * 60_000.0;
    let pauses: u64 = text
        .chars()
        .map(|c| match c {
            ',' => COMMA_MS,
            ':' | ';' => CLAUSE_MS,
            '.' | '!' | '?' => SENTENCE_MS,
            _ => 0,
        })
        .sum();
    ((speech + pauses as f64) / speed).round() as u64
}

impl DurationEstimator for WpmEstimator {
    fn estimate_ms(&self, req: &SynthRequest) -> u64 {
        estimate_ms(&req.text, self.wpm, req.speed)
    }
}
```

Have `null.rs` call `crate::estimator::estimate_ms`. Do **not** re-export it from the crate root: the moved tests never call it, and `WpmEstimator` is the public way to ask for an estimate.

- [ ] **Step 5: Update the consumers' imports**

Add `teleprompt-voice-null = { path = "../teleprompt-voice-null" }` to `teleprompt-cli`'s `[dependencies]` and to `teleprompt-compile`'s `[dev-dependencies]`. Change every `use teleprompt_voice::NullVoice` to `use teleprompt_voice_null::NullVoice`, and `teleprompt_voice::NULL_SAMPLE_RATE` likewise.

Move `crates/teleprompt-voice/tests/null.rs` to `crates/teleprompt-voice-null/tests/null.rs` and fix its imports the same way. Leave `crates/teleprompt-voice/tests/wav.rs` and `ladder.rs` where they are — they test the contract crate.

- [ ] **Step 6: Run everything**

Run: `cargo test --workspace`
Expected: PASS, with the same total as before plus the 5 new estimator tests.

Run: `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(voice): move the null backend out of the contract crate"
```

---

### Task 2: `teleprompt-cache` — content-addressed synthesis cache

Standalone and pure. Nothing is wired to it in this task; it is tested alone.

**Files:**
- Create: `crates/teleprompt-cache/Cargo.toml`, `crates/teleprompt-cache/src/lib.rs`
- Modify: `crates/teleprompt-voice/src/contract.rs` (serde derives on `WordTiming` — see Step 3)
- Test: `crates/teleprompt-cache/tests/cache.rs`

**Interfaces:**
- Consumes: `teleprompt_voice::{SynthRequest, Pcm, WordTiming, wav}`, `teleprompt_core::Hash`.
- Produces:
  - `pub struct VoiceCache { root: PathBuf }`, `VoiceCache::new(root: impl Into<PathBuf>) -> Self`
  - `pub struct CacheKey(Hash)` with `Display` giving 64 hex chars
  - `pub fn key(backend_id: &str, backend_version: &str, req: &SynthRequest) -> CacheKey`
  - `pub struct CachedAudio { pub duration_ms: u64, pub sample_rate: u32, pub channels: u16, pub word_timings: Option<Vec<WordTiming>>, pub wav: Vec<u8> }`
  - `VoiceCache::lookup(&self, key: &CacheKey) -> Result<Option<CachedAudio>, CacheError>`
  - `VoiceCache::store(&self, key: &CacheKey, pcm: &Pcm, word_timings: Option<&[WordTiming]>) -> Result<CachedAudio, CacheError>`
  - `VoiceCache::stats(&self) -> Result<CacheStats, CacheError>` with `CacheStats { entries: usize, bytes: u64 }`

- [ ] **Step 1: Write the failing test**

Create `crates/teleprompt-cache/tests/cache.rs`:

```rust
use teleprompt_cache::{key, VoiceCache};
use teleprompt_voice::{Pcm, SynthRequest, WordTiming};

fn req(text: &str, voice: Option<&str>, speed: f64, locale: &str) -> SynthRequest {
    SynthRequest {
        text: text.to_string(),
        locale: locale.to_string(),
        voice: voice.map(str::to_string),
        speed,
    }
}

fn pcm(ms: u64) -> Pcm {
    Pcm {
        sample_rate: 24_000,
        channels: 1,
        samples: vec![0; (ms * 24) as usize],
    }
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "tp-cache-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_miss_is_none_not_an_error() {
    let c = VoiceCache::new(tempdir("miss"));
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));
    assert!(c.lookup(&k).unwrap().is_none());
}

#[test]
fn store_then_lookup_round_trips() {
    let c = VoiceCache::new(tempdir("round"));
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));

    let stored = c.store(&k, &pcm(1000), None).unwrap();
    assert_eq!(stored.duration_ms, 1000);
    assert_eq!(stored.sample_rate, 24_000);
    assert_eq!(stored.channels, 1);
    assert_eq!(&stored.wav[0..4], b"RIFF");

    let got = c.lookup(&k).unwrap().expect("hit");
    assert_eq!(got.duration_ms, 1000);
    assert_eq!(got.sample_rate, 24_000);
    assert_eq!(got.channels, 1);
    assert_eq!(got.wav, stored.wav);
    assert!(got.word_timings.is_none());
}

#[test]
fn word_timings_survive_the_round_trip() {
    let c = VoiceCache::new(tempdir("words"));
    let k = key("null", "0.1.0", &req("hello there", None, 1.0, "en"));
    let timings = vec![
        WordTiming { word: "hello".into(), start_ms: 0, end_ms: 400 },
        WordTiming { word: "there".into(), start_ms: 400, end_ms: 900 },
    ];

    c.store(&k, &pcm(900), Some(&timings)).unwrap();
    assert_eq!(c.lookup(&k).unwrap().unwrap().word_timings.unwrap(), timings);
}

/// Every input that changes the audio must change the key. Varied one at a
/// time, because a key that ignores one field serves the wrong voice's audio
/// for another and nothing would ever notice.
#[test]
fn every_input_that_changes_the_audio_changes_the_key() {
    let base = key("null", "0.1.0", &req("hello", Some("af_heart"), 1.0, "en"));

    let variants = [
        ("backend id",      key("kokoro", "0.1.0", &req("hello", Some("af_heart"), 1.0, "en"))),
        ("backend version", key("null", "0.2.0", &req("hello", Some("af_heart"), 1.0, "en"))),
        ("text",            key("null", "0.1.0", &req("goodbye", Some("af_heart"), 1.0, "en"))),
        ("voice",           key("null", "0.1.0", &req("hello", Some("af_bella"), 1.0, "en"))),
        ("no voice",        key("null", "0.1.0", &req("hello", None, 1.0, "en"))),
        ("speed",           key("null", "0.1.0", &req("hello", Some("af_heart"), 2.0, "en"))),
        ("locale",          key("null", "0.1.0", &req("hello", Some("af_heart"), 1.0, "nl"))),
    ];

    for (what, k) in variants {
        assert_ne!(base.to_string(), k.to_string(), "{what} must change the key");
    }
}

#[test]
fn the_key_is_stable_across_calls() {
    let a = key("null", "0.1.0", &req("hello", Some("af_heart"), 1.0, "en"));
    let b = key("null", "0.1.0", &req("hello", Some("af_heart"), 1.0, "en"));
    assert_eq!(a.to_string(), b.to_string());
    assert_eq!(a.to_string().len(), 64, "64 hex characters");
}

#[test]
fn a_corrupt_sidecar_is_an_error_not_a_panic() {
    let root = tempdir("corrupt");
    let c = VoiceCache::new(&root);
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));
    c.store(&k, &pcm(1000), None).unwrap();

    std::fs::write(root.join(format!("voice/{k}.json")), "{ not json").unwrap();
    assert!(c.lookup(&k).is_err());
}

/// A sidecar with no audio beside it is a half-written entry, not a hit.
#[test]
fn a_sidecar_without_its_wav_is_a_miss() {
    let root = tempdir("halfwritten");
    let c = VoiceCache::new(&root);
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));
    c.store(&k, &pcm(1000), None).unwrap();

    std::fs::remove_file(root.join(format!("voice/{k}.wav"))).unwrap();
    assert!(c.lookup(&k).unwrap().is_none(), "treat as absent and re-synthesize");
}

#[test]
fn stats_count_entries_and_bytes() {
    let c = VoiceCache::new(tempdir("stats"));
    assert_eq!(c.stats().unwrap().entries, 0);

    c.store(&key("null", "0.1.0", &req("a", None, 1.0, "en")), &pcm(500), None).unwrap();
    c.store(&key("null", "0.1.0", &req("b", None, 1.0, "en")), &pcm(500), None).unwrap();

    let s = c.stats().unwrap();
    assert_eq!(s.entries, 2);
    assert!(s.bytes > 0);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p teleprompt-cache`
Expected: FAIL — the package does not exist.

- [ ] **Step 3: Create the crate**

`crates/teleprompt-cache/Cargo.toml`:

```toml
[package]
name = "teleprompt-cache"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
teleprompt-core = { path = "../teleprompt-core" }
teleprompt-voice = { path = "../teleprompt-voice" }
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
```

`crates/teleprompt-cache/src/lib.rs`:

```rust
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
                return Ok(CacheStats { entries: 0, bytes: 0 })
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
        Ok(CacheStats { entries: count, bytes })
    }
}

fn read_err(path: &Path, source: std::io::Error) -> CacheError {
    CacheError::Read { path: path.display().to_string(), source }
}

fn write_err(path: &Path, source: std::io::Error) -> CacheError {
    CacheError::Write { path: path.display().to_string(), source }
}
```

`WordTiming` currently derives `Debug, Clone, PartialEq, Eq` and nothing else — `crates/teleprompt-voice/src/contract.rs` imports no serde at all. Add `Serialize, Deserialize` to that one type (`serde` is already a dependency of `teleprompt-voice`). Leave every other type in that file alone: the cache is the only reason `WordTiming` needs to be serializable, and nothing else in the contract is being persisted.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p teleprompt-cache`
Expected: PASS, 8 tests.

Run: `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`
Expected: all clean.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(cache): content-addressed synthesis cache"
```

---

### Task 3: `duration_source` on narration

The timeline learns to say whether a narration duration was measured or predicted. Behaviour does not change yet — everything is still `Measured`, because `compile` still synthesizes. Task 4 makes it honest.

**Files:**
- Modify: `crates/teleprompt-schedule/src/beat.rs`, `crates/teleprompt-schedule/src/timeline.rs`, `crates/teleprompt-schedule/src/schedule.rs`
- Modify: `crates/teleprompt-compile/src/lib.rs`, `crates/teleprompt-compile/src/manifest.rs`
- Test: `crates/teleprompt-schedule/tests/schedule.rs`, `crates/teleprompt-schedule/tests/diff.rs`
- Snapshots: both `.snap` files change

**Every literal `NarrationInput { .. }` needs the new field.** There are three outside the struct definition: `crates/teleprompt-compile/src/lib.rs`, `crates/teleprompt-schedule/tests/schedule.rs`, and `crates/teleprompt-schedule/tests/diff.rs` (its `beat()` helper, around line 17). Default both test helpers to `DurationSource::Measured`; the compile site is covered in Step 4.

**Interfaces:**
- Consumes: `DurationSource` from `crates/teleprompt-schedule/src/beat.rs` — it already exists with variants `Exact`, `Estimated`, `Measured`.
- Produces: `NarrationInput.duration_source: DurationSource`; `NarrationEntry.duration_source: String`; `SegmentEntry.duration_source: String` on the manifest.

- [ ] **Step 1: Write the failing test**

Append to `crates/teleprompt-schedule/tests/schedule.rs`:

```rust
#[test]
fn narration_entries_report_where_their_duration_came_from() {
    let mut beats = vec![narration_beat("one", 5000), narration_beat("two", 3000)];
    beats[0].narration.as_mut().unwrap().duration_source = DurationSource::Measured;
    beats[1].narration.as_mut().unwrap().duration_source = DurationSource::Estimated;

    let (t, _) = schedule(&beats, "s.md", "en", "0.1.0");
    let sources: Vec<&str> = t
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .map(|n| n.duration_source.as_str())
        .collect();

    assert_eq!(
        sources,
        vec!["measured", "estimated"],
        "a reader must be able to tell a measurement from a prediction"
    );
}
```

Add `DurationSource` to that file's `use teleprompt_schedule::{...}` list, and have the `narration()` helper default the new field to `DurationSource::Measured`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p teleprompt-schedule --test schedule`
Expected: FAIL to compile — no field `duration_source` on `NarrationInput`.

- [ ] **Step 3: Add the field to the inputs and the entry**

In `crates/teleprompt-schedule/src/beat.rs`, add to `NarrationInput` after `duration_ms`:

```rust
    /// Whether `duration_ms` was measured from real audio or predicted.
    ///
    /// `plan` and `diff` never synthesize, so a segment not yet in the cache
    /// carries an estimate. Publishing which is which is the difference
    /// between a timeline a reader can trust and one that quietly conflates
    /// a prediction with a measurement.
    pub duration_source: DurationSource,
```

In `crates/teleprompt-schedule/src/timeline.rs`, add to `NarrationEntry` after `duration_ms`:

```rust
    pub duration_source: String,
```

In `crates/teleprompt-schedule/src/schedule.rs`, populate it in the `NarrationEntry` construction, reusing the same mapping the action arm already uses:

```rust
                duration_source: match n.duration_source {
                    DurationSource::Exact => "exact",
                    DurationSource::Estimated => "estimated",
                    DurationSource::Measured => "measured",
                }
                .to_string(),
```

- [ ] **Step 4: Set it at the one construction site, and publish it**

In `crates/teleprompt-compile/src/lib.rs`, add `duration_source: DurationSource::Measured` to the `NarrationInput` the narration arm builds — `compile` still synthesizes, so that is honest for now.

Expect these same snapshots to change once more in Task 4, when `compile` starts reading a cold cache and the value becomes `estimated`. That is intended, not a mistake to correct here. The *durations* will not move: `NullVoice::synthesize` and `WpmEstimator::estimate_ms` call the same underlying function, so only `duration_source` differs between the two commits. Import `DurationSource` from `teleprompt_schedule`.

In `crates/teleprompt-compile/src/manifest.rs`, add to `SegmentEntry` after `duration_ms`:

```rust
    /// `measured` when this came from real audio, `estimated` when it is the
    /// duration model's prediction. A consumer building a player can show the
    /// difference; one committing the manifest should know a re-dub will move
    /// every estimated segment.
    pub duration_source: String,
```

and populate it in `build()` from `n.duration_source.clone()`.

Update the spec's §5 example and §5.1 field table in
`docs/superpowers/specs/2026-08-15-teleprompt-narration-manifest-design.md` to include the field.

- [ ] **Step 5: Run the tests and accept the snapshots**

Run: `cargo test --workspace`
Expected: two snapshot failures, both adding `duration_source: "measured"`.

Review each `.snap.new` by eye — confirm the only change is the added field — then rename to `.snap`. `cargo-insta` is not installed in this environment; renaming is the documented fallback.

Run: `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`
Expected: all clean.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(schedule): narration says whether its duration was measured"
```

---

### Task 4: Get the backend off the hot path

The centre of the plan. `compile()` stops taking a `&dyn VoiceBackend` and starts taking a cache and an estimator. After this, `check`, `plan`, and `diff` cannot call a backend even by accident — the type does not let them.

**Files:**
- Modify: `crates/teleprompt-compile/src/lib.rs`, `crates/teleprompt-compile/Cargo.toml`
- Modify: `crates/teleprompt-cli/src/cmd/check.rs`, `crates/teleprompt-cli/src/cmd/dub.rs`
- Test: `crates/teleprompt-compile/tests/compile.rs`

**Interfaces:**
- Consumes: `VoiceCache`, `key`, `CachedAudio` (Task 2); `DurationEstimator` (Task 1); `DurationSource` (Task 3).
- Produces:
  - `compile(program, registry, voice_ctx: &VoiceContext, base_dir, version) -> Result<CompileOutput, Diagnostics>`
  - `pub struct VoiceContext<'a> { pub backend_id: &'a str, pub backend_version: &'a str, pub cache: &'a VoiceCache, pub estimator: &'a dyn DurationEstimator }`
  - `NarrationDetail.cache_key: CacheKey` — so `dub` stores under the same key `compile` looked up.

**Where `audio_hash` now comes from.** `NarrationEntry.audio_hash` used to be the backend's synthesis result. With no synthesis on this path, it becomes **the cache key** — the identity of the audio this segment resolves to, whether or not it has been rendered. It is stable, it changes when the audio would change, and it is exactly what a drift check wants. The manifest's `audio_hash` is unaffected: `dub` already overwrites it with a hash of the bytes actually written, and the two are documented as different things.

- [ ] **Step 1: Write the failing test**

Append to `crates/teleprompt-compile/tests/compile.rs`:

```rust
use teleprompt_cache::VoiceCache;
use teleprompt_compile::VoiceContext;
use teleprompt_voice::Pcm;
use teleprompt_voice_null::WpmEstimator;

fn cache_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "tp-compile-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

const ONE: &str = "# Quick start\n\nOne two three four five six.\n";

#[test]
fn a_cold_cache_yields_estimated_durations() {
    let cache = VoiceCache::new(cache_dir("cold"));
    let est = WpmEstimator::default();
    let ctx = VoiceContext {
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &est,
    };
    let out = compile_with(ONE, &ctx).expect("compiles");

    let n = out.timeline.entries[0].narration.as_ref().unwrap();
    assert_eq!(n.duration_source, "estimated");
    assert_eq!(n.duration_ms, 2400, "six words at 150 wpm");
}

#[test]
fn a_warm_cache_yields_measured_durations() {
    let cache = VoiceCache::new(cache_dir("warm"));
    let est = WpmEstimator::default();
    let ctx = VoiceContext {
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &est,
    };

    // Populate the cache under the key compile will look up, with audio of a
    // deliberately different length from the estimate.
    let cold = compile_with(ONE, &ctx).expect("compiles");
    let k = cold.narration[0].cache_key.clone();
    cache
        .store(
            &k,
            &Pcm { sample_rate: 24_000, channels: 1, samples: vec![0; 24_000 * 5] },
            None,
        )
        .unwrap();

    let warm = compile_with(ONE, &ctx).expect("compiles");
    let n = warm.timeline.entries[0].narration.as_ref().unwrap();
    assert_eq!(n.duration_source, "measured");
    assert_eq!(n.duration_ms, 5000, "the cached audio's real length, not the estimate");
}

#[test]
fn the_cache_key_covers_the_resolved_voice_config() {
    let cache = VoiceCache::new(cache_dir("cfgkey"));
    let est = WpmEstimator::default();
    let ctx = VoiceContext {
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &est,
    };

    let plain = compile_with(ONE, &ctx).unwrap();
    let fast = compile_with(
        "---\nvoice: { speed: 2.0 }\n---\n\n# Quick start\n\nOne two three four five six.\n",
        &ctx,
    )
    .unwrap();

    assert_ne!(
        plain.narration[0].cache_key.to_string(),
        fast.narration[0].cache_key.to_string(),
        "a different speed is different audio and must not share a cache entry"
    );
}
```

Add a `compile_with` helper beside the existing `compile_str`, taking the context; keep `compile_str` as a thin wrapper over it that builds a throwaway cache so the older tests are untouched.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p teleprompt-compile --test compile`
Expected: FAIL to compile — `VoiceContext` not found.

- [ ] **Step 3: Change the signature**

Add `teleprompt-cache = { path = "../teleprompt-cache" }` to `teleprompt-compile`'s `[dependencies]` and `teleprompt-voice-null` to its `[dev-dependencies]`.

In `crates/teleprompt-compile/src/lib.rs`:

```rust
/// Everything `compile` needs about voice — and deliberately not a backend.
///
/// `compile` runs on the inner loop: `check`, `plan`, and `diff` call it on
/// every run and must stay sub-second and offline. Passing a `VoiceBackend`
/// here is what would make that impossible, so the type simply does not
/// offer one. Audio is `dub`'s and `build`'s business.
pub struct VoiceContext<'a> {
    pub backend_id: &'a str,
    pub backend_version: &'a str,
    pub cache: &'a VoiceCache,
    pub estimator: &'a dyn DurationEstimator,
}
```

Replace the `voice: &dyn VoiceBackend` parameter with `voice_ctx: &VoiceContext`, and replace the synthesize block in the narration arm:

```rust
                let req = SynthRequest {
                    text: text.clone(),
                    locale: program.locale.clone(),
                    voice: config.voice.voice.clone(),
                    speed: config.voice.speed,
                };
                let cache_key = teleprompt_cache::key(
                    voice_ctx.backend_id,
                    voice_ctx.backend_version,
                    &req,
                );

                let cached = match voice_ctx.cache.lookup(&cache_key) {
                    Ok(c) => c,
                    Err(e) => {
                        diags.push(Diagnostic::error(format!("segment `{id}`: {e}")));
                        continue;
                    }
                };
                let (duration_ms, duration_source, word_timings) = match &cached {
                    Some(hit) => (
                        hit.duration_ms,
                        DurationSource::Measured,
                        hit.word_timings.clone(),
                    ),
                    None => (
                        voice_ctx.estimator.estimate_ms(&req),
                        DurationSource::Estimated,
                        None,
                    ),
                };
```

`NarrationDetail` gains `pub cache_key: CacheKey` and keeps `synth_request`; both are what `dub` needs to render and store. `NarrationInput` takes `duration_ms`, `duration_source`, and `audio_hash: Hash::of(cache_key.to_string().as_bytes())`.

Document the `audio_hash` change at the field:

```rust
                    // The cache key, not a synthesis result: this path never
                    // synthesizes. It identifies the audio this segment
                    // resolves to, so it moves exactly when the audio would.
                    // The manifest's `audio_hash` is a different thing — a
                    // hash of the bytes `dub` actually wrote.
                    audio_hash: Hash::of(cache_key.to_string().as_bytes()),
```

- [ ] **Step 4: Update the two callers**

`crates/teleprompt-cli/src/cmd/check.rs`'s `compile_script` builds the context from the project: cache rooted at `<project root>/.teleprompt/cache`, `WpmEstimator::default()`, and `backend_id` from `project.config.voice.backend`. Task 5 makes the id validated; for now pass it through and use `"0.1.0"` for the version, with a comment that Task 5 replaces it with the registry's reported version.

`crates/teleprompt-cli/src/cmd/dub.rs` no longer gets durations from a backend. Leave its `render_pcm` loop working for now by having it look up the cache first and synthesize on a miss — Task 6 rewrites this properly against the async trait. Keep the existing WAV-length guard.

- [ ] **Step 5: Run the tests**

Run: `cargo test --workspace`
Expected: PASS. Existing snapshots will change — every narration goes from `measured` to `estimated` in tests that use a cold cache. Review the `.snap.new` files by eye and accept.

Run `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`.

- [ ] **Step 6: Verify the property by hand**

```bash
cargo build
cd $(mktemp -d) && cargo run --manifest-path <repo>/Cargo.toml -- new p >/dev/null
cd p && printf '# A\n\nOne two three four five six.\n' > scripts/a.md
time cargo run --manifest-path <repo>/Cargo.toml -- plan scripts/a.md
```

Expected: `estimated` in the JSON, and a runtime dominated by process start, not synthesis. Put the output in the report.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(compile): read durations from the cache, never from a backend"
```

---

### Task 5: Registry and config-driven selection

**Files:**
- Create: `crates/teleprompt-voice/src/registry.rs`, `crates/teleprompt-cli/src/voice.rs`
- Modify: `crates/teleprompt-voice/src/lib.rs`, `crates/teleprompt-cli/src/lib.rs`, `crates/teleprompt-cli/src/cmd/check.rs`, `crates/teleprompt-cli/src/cmd/doctor.rs`
- Test: `crates/teleprompt-voice/tests/registry.rs`, `crates/teleprompt-cli/tests/commands.rs`

**Interfaces:**
- Produces:
  - `VoiceRegistry::default()`, `register(Arc<dyn VoiceBackend>)`, `get(&str) -> Option<Arc<dyn VoiceBackend>>`, `available() -> Vec<&str>`
  - `teleprompt_cli::voice::registry() -> VoiceRegistry` — the wired set
  - `teleprompt_cli::voice::resolve(&VoiceRegistry, &str) -> Result<Arc<dyn VoiceBackend>, String>`

- [ ] **Step 1: Write the failing test**

Create `crates/teleprompt-voice/tests/registry.rs`:

```rust
use std::sync::Arc;
use teleprompt_voice::{VoiceBackend, VoiceRegistry};

mod stub;
use stub::StubVoice;

#[test]
fn available_is_sorted_so_doctor_output_is_deterministic() {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(StubVoice::new("zulu")));
    r.register(Arc::new(StubVoice::new("alpha")));
    r.register(Arc::new(StubVoice::new("mike")));
    assert_eq!(r.available(), vec!["alpha", "mike", "zulu"]);
}

#[test]
fn get_returns_the_registered_backend_and_none_otherwise() {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(StubVoice::new("alpha")));
    assert_eq!(r.get("alpha").map(|b| b.id().to_string()), Some("alpha".to_string()));
    assert!(r.get("missing").is_none());
}

#[test]
fn registering_the_same_id_twice_replaces_it() {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(StubVoice::new("alpha")));
    r.register(Arc::new(StubVoice::new("alpha")));
    assert_eq!(r.available(), vec!["alpha"], "one entry, not two");
}
```

Create `crates/teleprompt-voice/tests/stub/mod.rs` with a minimal `StubVoice` implementing the trait **as it stands at this task — still synchronous**, returning an error from `synthesize`; the registry does not care what it synthesizes. Task 6 converts it along with the trait, and names this file.

Append to `crates/teleprompt-cli/tests/commands.rs`:

```rust
#[test]
fn an_unknown_voice_backend_is_a_validation_error_naming_what_exists() {
    let (p, s) = project_with(
        "---\nvoice: { backend: nope }\n---\n\n# Intro\n\nOne two three. {#a}\n",
    );
    let errors = run_check(&p, &s, "en").expect_err("unknown backend must fail check");
    let joined = errors.join("\n");
    assert!(joined.contains("nope"), "{joined}");
    assert!(joined.contains("null"), "must name what is available: {joined}");
}

#[test]
fn the_default_backend_is_null_so_existing_scripts_keep_working() {
    let (p, s) = project_with("# Intro\n\nOne two three. {#a}\n");
    assert!(run_check(&p, &s, "en").is_ok());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p teleprompt-voice --test registry` → FAIL, `VoiceRegistry` not found.
Run: `cargo test -p teleprompt-cli --test commands` → FAIL, unknown backend is currently accepted.

- [ ] **Step 3: Write the registry**

`crates/teleprompt-voice/src/registry.rs`:

```rust
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::contract::VoiceBackend;

/// Mirrors `SceneRegistry`. `BTreeMap` so `available()` is ordered and
/// `doctor`'s output does not depend on insertion order.
///
/// `Arc`, not `Box`: `dub` synthesizes segments concurrently, so the backend
/// is shared across tasks.
#[derive(Default)]
pub struct VoiceRegistry {
    backends: BTreeMap<String, Arc<dyn VoiceBackend>>,
}

impl VoiceRegistry {
    pub fn register(&mut self, backend: Arc<dyn VoiceBackend>) {
        self.backends.insert(backend.id().to_string(), backend);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn VoiceBackend>> {
        self.backends.get(id).cloned()
    }

    pub fn available(&self) -> Vec<&str> {
        self.backends.keys().map(String::as_str).collect()
    }
}
```

Add `pub mod registry;` and `pub use registry::VoiceRegistry;` to the crate root.

- [ ] **Step 4: Wire the CLI**

`crates/teleprompt-cli/src/voice.rs`:

```rust
use std::sync::Arc;

use teleprompt_voice::{VoiceBackend, VoiceRegistry};
use teleprompt_voice_null::NullVoice;

/// The backends this build ships. Adding one is a line here plus a crate —
/// nothing else in the workspace changes.
pub fn registry() -> VoiceRegistry {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(NullVoice::default()));
    r
}

pub fn resolve(
    registry: &VoiceRegistry,
    id: &str,
) -> Result<Arc<dyn VoiceBackend>, String> {
    registry.get(id).ok_or_else(|| {
        format!(
            "unknown voice backend `{id}` (available: {})",
            registry.available().join(", ")
        )
    })
}
```

Add `pub mod voice;` to `crates/teleprompt-cli/src/lib.rs`. In `compile_script`, resolve `project.config.voice.backend` through it before compiling, return the error as a validation error, and take `backend_id` and `backend_version` for the `VoiceContext` from the resolved backend's `id()` and `capabilities().version`.

`VoiceCapabilities` gains `pub version: String`; `NullVoice` reports `env!("CARGO_PKG_VERSION")` of its own crate.

- [ ] **Step 5: Report it in `doctor`**

In `crates/teleprompt-cli/src/cmd/doctor.rs`, replace the hardcoded `vec!["null".to_string()]` with `crate::voice::registry().available()`, and add a cache line:

```rust
    let cache = teleprompt_cache::VoiceCache::new(".teleprompt/cache");
    let stats = cache.stats().unwrap_or(teleprompt_cache::CacheStats { entries: 0, bytes: 0 });
```

rendered as `cache            .teleprompt/cache/voice — {entries} entries, {bytes} bytes`.

- [ ] **Step 6: Run everything and commit**

Run: `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`
Expected: all clean. Update the doctor assertions in `crates/teleprompt-cli/tests/new.rs` if they enumerate lines.

```bash
git add -A
git commit -m "feat(voice): registry, and voice.backend finally means something"
```

---

### Task 6: The async contract

Only now, with `compile` off the backend, does making the trait async touch anything but `dub`.

**Files:**
- Modify: `crates/teleprompt-voice/src/contract.rs`, `crates/teleprompt-voice/Cargo.toml`
- Modify: `crates/teleprompt-voice-null/src/null.rs`, `crates/teleprompt-voice-null/Cargo.toml`
- Modify: `crates/teleprompt-cli/src/cmd/dub.rs`, `crates/teleprompt-cli/src/main.rs`, `crates/teleprompt-cli/Cargo.toml`
- Modify: root `Cargo.toml` (workspace deps)
- Test: `crates/teleprompt-voice-null/tests/null.rs`, `crates/teleprompt-cli/tests/dub.rs`

**Two test doubles also implement `VoiceBackend` and must be converted with it**, or the workspace will not compile: `crates/teleprompt-voice/tests/stub/mod.rs` (`StubVoice`, from Task 5) and `crates/teleprompt-compile/tests/manifest.rs` (`WordyVoice`, which exists to cover the word-timing path). Both become `#[async_trait::async_trait]` with `async fn synthesize` returning `Synthesized`. `WordyVoice` currently delegates to a `NullVoice` and overrides `word_timings`; keep that shape.

**Interfaces:**
- Produces:
  - `async fn VoiceBackend::synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError>`
  - `pub struct Synthesized { pub pcm: Pcm, pub word_timings: Option<Vec<WordTiming>> }`
  - `SynthResult`, `render_pcm`, and `cache_key` are **removed**.

- [ ] **Step 1: Write the failing test**

Replace the `render_pcm` tests in `crates/teleprompt-voice-null/tests/null.rs` with:

```rust
#[tokio::test]
async fn null_synthesizes_silence_matching_the_estimate() {
    let v = NullVoice::default();
    let r = req("Every video in this repository is built from a script you can read.");

    let out = v.synthesize(&r).await.unwrap();

    assert_eq!(out.pcm.sample_rate, NULL_SAMPLE_RATE);
    assert_eq!(out.pcm.channels, 1);
    assert!(out.pcm.samples.iter().all(|s| *s == 0), "null is silence");
    assert_eq!(
        out.pcm.duration_ms(),
        WpmEstimator::default().estimate_ms(&r),
        "the audio's length is the estimate — one number, not two"
    );
    assert!(out.word_timings.is_none());
}

#[tokio::test]
async fn null_rejects_a_non_positive_speed() {
    let v = NullVoice::default();
    let mut r = req("hello");
    r.speed = 0.0;
    assert!(v.synthesize(&r).await.is_err());
}

#[tokio::test]
async fn empty_text_synthesizes_nothing() {
    let v = NullVoice::default();
    let out = v.synthesize(&req("   ")).await.unwrap();
    assert_eq!(out.pcm.samples.len(), 0);
    assert_eq!(out.pcm.duration_ms(), 0);
}
```

Add `tokio = { workspace = true, features = ["macros", "rt"] }` to `teleprompt-voice-null`'s `[dev-dependencies]`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p teleprompt-voice-null`
Expected: FAIL — `synthesize` is not async and returns `SynthResult`.

- [ ] **Step 3: Add the workspace dependencies**

In the root `Cargo.toml` `[workspace.dependencies]`:

```toml
async-trait = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

- [ ] **Step 4: Sharpen the trait**

In `crates/teleprompt-voice/src/contract.rs`, delete `SynthResult`, and replace the three methods:

```rust
/// What a backend produces. Duration is deliberately absent: it is
/// `pcm.duration_ms()`, so a backend cannot report a length its samples do
/// not have. An earlier contract returned the two separately, and `dub`
/// published a 3250 ms duration beside a 6500 ms file.
#[derive(Debug, Clone)]
pub struct Synthesized {
    pub pcm: Pcm,
    /// Present only when `capabilities().word_timings` is true.
    pub word_timings: Option<Vec<WordTiming>>,
}

#[async_trait::async_trait]
pub trait VoiceBackend: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> VoiceCapabilities;

    /// Turn text into audio. The only thing a backend does.
    ///
    /// Caching and duration prediction are teleprompt's concerns and appear
    /// nowhere in this contract.
    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError>;
}
```

Add `async-trait.workspace = true` to `teleprompt-voice`'s dependencies. Update `NullVoice` to match, using `crate::estimator::estimate_ms` for the length.

- [ ] **Step 5: Make `dub` async**

`main.rs` gets `#[tokio::main]` and `async fn main`. `run_dub` becomes `async fn` and, per segment: compute the key (it is already on `NarrationDetail`), look it up, and on a miss `backend.synthesize(&detail.synth_request).await` then `cache.store(...)`. Keep the existing guard that the WAV's length equals the manifest's `duration_ms`, and keep rendering everything into memory before any file is written.

Add `tokio.workspace = true` to `teleprompt-cli`'s dependencies.

**Do not make `check`, `plan`, or `diff` async.** They call `compile`, which is synchronous, and `#[tokio::main]` on `main` does not change that. A reviewer will check this.

- [ ] **Step 6: Run everything**

Run: `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`
Expected: all clean.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(voice): one async method, returning audio"
```

---

### Task 7: `now measured` as a diff reason

The first `dub` after writing a segment turns a prediction into a measurement, which moves the timeline and fails `diff --exit-code` — correctly, because the video changed. It must not read as a content edit.

**Files:**
- Modify: `crates/teleprompt-schedule/src/diff.rs`
- Test: `crates/teleprompt-schedule/tests/diff.rs`

**Interfaces:**
- Consumes: `NarrationEntry.duration_source` (Task 3).
- Produces: a `now measured` reason in the beat-change classification.

- [ ] **Step 1: Write the failing test**

Append to `crates/teleprompt-schedule/tests/diff.rs`:

```rust
/// The estimated-to-measured transition is real drift — the video did change
/// — but it is not an author's edit, and conflating the two would send a
/// reader to re-read prose that nobody touched.
#[test]
fn an_estimate_becoming_a_measurement_is_named_as_such() {
    let before = {
        let mut b = beat("b1", 4000, "one");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Estimated;
        timeline(vec![b])
    };
    let after = {
        let mut b = beat("b1", 4300, "one");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Measured;
        timeline(vec![b])
    };

    let d = diff(&before, &after);
    assert!(!d.is_empty());
    let changed = d.changed.iter().find(|c| c.beat == "b1").expect("b1 changed");
    assert_eq!(changed.reason, "now measured");

    let rendered = d.render();
    assert!(rendered.contains("now measured"), "{rendered}");
}

/// A text edit that also happens to cross the estimated/measured boundary is
/// still a text edit — that is the cause the author can act on.
#[test]
fn a_text_edit_outranks_the_measurement_transition() {
    let before = {
        let mut b = beat("b1", 4000, "one");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Estimated;
        timeline(vec![b])
    };
    let after = {
        let mut b = beat("b1", 4300, "two");
        b.narration.as_mut().unwrap().duration_source = DurationSource::Measured;
        timeline(vec![b])
    };

    let d = diff(&before, &after);
    assert_eq!(d.changed[0].reason, "text edited");
}
```

The `beat` helper's third argument is the text the `source_hash` is derived from; check the existing helper and match it.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p teleprompt-schedule --test diff`
Expected: FAIL — the reason is reported as an audio or duration change.

- [ ] **Step 3: Add the reason**

In `crates/teleprompt-schedule/src/diff.rs`, in the function that classifies a changed beat, insert the new arm **after** the `source_hash` check and **before** the audio/duration checks:

```rust
    // Ordered after `text edited` deliberately: an author's edit is the cause
    // they can act on, and it explains the duration change by itself. Ordered
    // before the audio and duration checks because those would otherwise
    // absorb this and report a cause that sends the reader to the wrong place.
    } else if before.duration_source != after.duration_source
        && after.duration_source == "measured"
    {
        Some("now measured".to_string())
```

Match the existing function's exact shape and naming; do not restructure it.

- [ ] **Step 4: Run and commit**

Run: `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add -A
git commit -m "feat(schedule): name the estimate-to-measurement transition"
```

---

### Task 8: Warn on committing estimates, docs, and acceptance

**Files:**
- Modify: `crates/teleprompt-cli/src/cmd/plan.rs`, `crates/teleprompt-cli/src/main.rs`
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-08-15-teleprompt-design.md` (§4.3)
- Test: `crates/teleprompt-cli/tests/voice_loop.rs` (create)

- [ ] **Step 1: Write the failing acceptance test**

Create `crates/teleprompt-cli/tests/voice_loop.rs`, copying the `tempdir` / `project_with` / `tp` / `code` helpers from `tests/dub.rs`:

```rust
const SCRIPT: &str = "\
# Quick start

Every video in this repository is built from a script you can read.
";

/// The whole point of Delivery A, end to end: plan is estimated and instant,
/// dub measures it, and plan afterwards is measured without becoming slow.
#[test]
fn plan_is_estimated_until_dub_measures_it() {
    let root = project_with("loop", SCRIPT);

    let first = tp(&root, &["--format", "json", "plan", "scripts/test.md"]);
    assert_eq!(code(&first), 0);
    let t: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(t["entries"][0]["narration"]["duration_source"], "estimated");

    assert_eq!(code(&tp(&root, &["dub", "scripts/test.md", "--out", "out"])), 0);

    let second = tp(&root, &["--format", "json", "plan", "scripts/test.md"]);
    let t2: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(t2["entries"][0]["narration"]["duration_source"], "measured");
}

#[test]
fn plan_warns_when_it_emits_estimated_durations() {
    let root = project_with("warn", SCRIPT);
    let out = tp(&root, &["plan", "scripts/test.md"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("estimated"),
        "a timeline committed from a cold cache drifts on the next dub: {stderr}"
    );
}

#[test]
fn the_cache_survives_across_runs_so_the_second_dub_is_a_no_op_diff() {
    let root = project_with("stable", SCRIPT);
    tp(&root, &["dub", "scripts/test.md", "--out", "out"]);
    let a = std::fs::read(root.join("out/en/narration.json")).unwrap();
    tp(&root, &["dub", "scripts/test.md", "--out", "out"]);
    let b = std::fs::read(root.join("out/en/narration.json")).unwrap();
    assert_eq!(a, b, "a warm cache must reproduce the manifest exactly");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p teleprompt-cli --test voice_loop`
Expected: FAIL — no warning on estimated durations.

- [ ] **Step 3: Emit the warning**

In `run_plan`'s caller in `main.rs`, after a successful plan, count narration entries whose `duration_source` is `"estimated"` and emit to stderr:

```
warning: 3 of 4 narration durations are estimated; run `teleprompt dub` to measure them before committing this timeline
```

Suppress it when the count is zero.

- [ ] **Step 4: Document it**

Add to `README.md`, after the commands table:

````markdown
### Estimated versus measured durations

`plan`, `check`, and `diff` never synthesize. They read durations from the
cache, and fall back to a word-count estimate for anything not yet rendered —
so the inner loop stays instant and offline no matter how slow the configured
voice is.

```
$ teleprompt plan scripts/tour.md --format json | grep duration_source
  "duration_source": "estimated"
```

`teleprompt dub` does the real synthesis and fills the cache. Afterwards the
same `plan` is still instant, and now says `measured`.

A timeline committed from a cold cache will drift the first time you `dub`,
which `diff` reports as `now measured` rather than as a content edit. `plan`
warns when it emits estimates for exactly that reason.
````

Update core spec §4.3's `null` bullet to say the word-count model now lives in
`teleprompt-voice-null` as the default `DurationEstimator`, and that `null` the
*backend* is silence of that length.

- [ ] **Step 5: Full verification**

Run: `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`

Then by hand, and put the real output in the report:

```bash
cd $(mktemp -d) && cargo run --manifest-path <repo>/Cargo.toml -- new p >/dev/null && cd p
cp <repo>/tests/fixtures/tour.md scripts/
cargo run --manifest-path <repo>/Cargo.toml -- plan scripts/tour.md
cargo run --manifest-path <repo>/Cargo.toml -- dub scripts/tour.md --out out
cargo run --manifest-path <repo>/Cargo.toml -- plan scripts/tour.md
cargo run --manifest-path <repo>/Cargo.toml -- doctor
```

Expected: estimated first, measured after the dub, and `doctor` reporting both the registry and a non-empty cache.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(cli): warn on committing estimates; document the loop"
```

---

## Self-Review

**Spec coverage.** §3 the contract → Task 6; §3.1 the estimator → Task 1; §4 crates → Tasks 1, 2, 6; §4.1 registry → Task 5; §4.2 selection → Task 5; §5 the cache → Task 2, wired in Task 4; §6 estimated vs measured → Tasks 3, 4; §6.1 the `now measured` reason and the `plan` warning → Tasks 7, 8; §8 the async boundary → Task 6, with the "do not make plan async" constraint stated in the task and in Global Constraints; §9 `doctor` → Task 5; §10 determinism → the cache-key test in Task 2 and the byte-stability test in Task 8; §11 testing → every task; §13 what breaks → Tasks 3, 4, 6 each carry their snapshot updates.

§7 (Kokoro) and §12 (runtime plugins) have no task, by design — §7 is Delivery B and §12 is deferred.

**One spec requirement I am deliberately deferring to Delivery B:** §9's `doctor` line probing a backend's reachability. With only `null` registered there is nothing to probe, and a probe API on the trait with no implementation that uses it would be exactly the speculative-interface mistake this spec exists to correct. Task 5 reports the registry and the cache; reachability arrives with the backend that can be unreachable.

**Placeholder scan.** No TBD, no "handle errors appropriately". Two tasks say "match the existing function's shape" rather than reproducing a function this plan does not change — Task 7's reason-classification arm and Task 5's doctor render — because inventing a second shape in the plan would be worse than reading the first.

**Type consistency.** `DurationEstimator` (T1) is consumed by `VoiceContext` (T4) and `WpmEstimator` (T1) satisfies it. `VoiceCache`/`key`/`CacheKey` (T2) are consumed by T4 and T6. `DurationSource` (existing, T3) flows to `NarrationEntry.duration_source: String` (T3) and is read by T7 as a string. `NarrationDetail.cache_key: CacheKey` (T4) is consumed by T6. `VoiceCapabilities.version` is added in T5 and read by T4's context — **note the ordering**: T4 passes a placeholder `"0.1.0"` and T5 replaces it with the real value, which is called out in T4 Step 4 so a reviewer does not read the placeholder as a defect.

**One risk the ordering creates.** Task 4 changes `audio_hash`'s meaning on the timeline before Task 6 removes the old trait methods, so between those commits `dub` has both a cache path and a `render_pcm` path. That is deliberate — it keeps the suite green at every commit — but it means Task 4's `dub` is transitional code that Task 6 deletes. Task 4 says so; a reviewer should not polish it.
