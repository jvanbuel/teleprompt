# Kokoro Voice Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Kokoro-FastAPI voice backend as a new crate plus one registration line, proving the contract Delivery A sharpened admits an implementation nothing in it was designed around.

**Architecture:** A new `teleprompt-voice-kokoro` crate implements `VoiceBackend` by POSTing to an OpenAI-compatible local HTTP server and decoding raw little-endian 16-bit PCM. Backend settings arrive through a generic `backends:` map in core config that core never interprets. `dub` gains bounded concurrent synthesis and per-segment progress; `doctor` gains a reachability probe. The `check`/`plan`/`diff` inner loop is untouched and must stay synchronous, offline, and audio-free.

**Tech Stack:** Rust 2021, MSRV 1.85, `async-trait`, `tokio` (current-thread runtime, CLI only), `reqwest` (new — `rustls-tls` + `json`, `default-features = false`), `serde`/`serde_json`/`serde_yaml`.

**Spec:** `docs/superpowers/specs/2026-08-15-teleprompt-voice-backends-design.md` (§7–§11, §14 Delivery B). Core design: `docs/superpowers/specs/2026-08-15-teleprompt-design.md`.

## Global Constraints

- **MSRV is 1.85.** CI runs `cargo check --workspace` on 1.85 (`.github/workflows/ci.yml`). Any new dependency must build on it.
- **The inner loop never synthesizes, never reads audio, and never awaits.** `check`, `plan`, `diff` must gain no `async fn`, no `.await`, no network call, and no tokio runtime. `teleprompt-compile` keeps `tokio` as a dev-dependency only.
- **`compile()` takes `VoiceContext`**, which is transitively backend-free. Do not add a backend, a client, or anything holding one to it.
- **Exit codes are issued only through `teleprompt_cli::output::exit_code_for`.** A panic reaching the user (exit 101) violates this.
- **A broken synthesizer is never a silent fallback** (spec §7.1). Unreachable, slow, or non-200 fails the command with exit 1, naming the URL and the segment. It does **not** fall back to `null`.
- **A backend whose output depends on configuration the `SynthRequest` does not carry must fold that configuration into `capabilities().version`** (documented on the field in `crates/teleprompt-voice/src/contract.rs`). For Kokoro that means the model and the host.
- **`teleprompt-voice-null` stays written against `teleprompt-voice`'s public API only.** If Kokoro needs something not exported, export it deliberately — do not reach around the contract.
- **No network dependency and no model download in the test suite** (spec §11). Kokoro is tested against an in-process stub HTTP server on an ephemeral port. Exactly one `#[ignore]`d test may hit a real server.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` must be green at every commit.
- Every behavioural change gets a test that fails before it and passes after. Run it and see it fail first.
- Do not put a model identifier in commit messages, code comments, or any pushed artifact.

---

## File Structure

**New crate `crates/teleprompt-voice-kokoro/`:**

| File | Responsibility |
|---|---|
| `Cargo.toml` | Depends on `teleprompt-voice`, `reqwest`, `serde`, `serde_json`, `serde_yaml`, `async-trait`. **Not** `teleprompt-core` — the crate must prove it needs only the contract. |
| `src/lib.rs` | Module wiring and public re-exports: `KokoroVoice`, `KokoroConfig`, `KOKORO_SAMPLE_RATE`. |
| `src/config.rs` | `KokoroConfig` — `base_url`, `timeout_ms`, `concurrency`, `model`. Deserializes from a `serde_yaml::Value`, with defaults. Owns `version_string()`. |
| `src/client.rs` | HTTP: `speech()` (POST, returns raw bytes), `voices()` (GET, returns `Vec<String>`). Owns error mapping and PCM decoding. |
| `src/backend.rs` | `KokoroVoice` — the `VoiceBackend` impl. Thin: capabilities plus a call into `client`. |
| `tests/stub/mod.rs` | Reusable in-process stub server: takes canned responses, returns a `base_url` and a handle recording received request bodies. |
| `tests/synth.rs` | Request body shape, PCM decode, 24 kHz `Pcm`, timeout, 500, truncated body, unreachable. |
| `tests/voices.rs` | `/v1/audio/voices` parsing and failure. |
| `tests/real.rs` | One `#[ignore]`d test against a real server. |

**Modified:**

| File | Change |
|---|---|
| `Cargo.toml` (workspace) | New member is automatic (`members = ["crates/*"]`). Add `reqwest` to `[workspace.dependencies]`. |
| `crates/teleprompt-core/src/config.rs` | `PartialConfig.backends` and `Config.backends`, both `BTreeMap<String, serde_yaml::Value>`. Core never interprets them. |
| `crates/teleprompt-cli/Cargo.toml` | Depend on `teleprompt-voice-kokoro`. |
| `crates/teleprompt-cli/src/voice.rs` | Register `KokoroVoice`. This is the "one line" the spec's §4.1 claim rests on. |
| `crates/teleprompt-cli/src/main.rs` | Extend the runtime to the `Doctor` arm. `check`/`plan`/`diff`/`new` stay runtime-free. |
| `crates/teleprompt-cli/src/cmd/doctor.rs` | Async probe of the configured backend; unreachable is a warning, not an error. |
| `crates/teleprompt-cli/src/cmd/dub.rs` | Validate the voice once before synthesis; bounded concurrent synthesis; per-segment progress. |
| `README.md`, spec §7/§9 | Document `backends:`, the concurrency setting, and the cache/host coupling. |

---

## Task 1: A generic `backends:` config map

Core must carry backend settings without naming any backend. A `kokoro:` field in `teleprompt-core::Config` would make the pluggability claim false on the first backend.

**Files:**
- Modify: `crates/teleprompt-core/src/config.rs`
- Test: `crates/teleprompt-core/tests/config.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `Config.backends: BTreeMap<String, serde_yaml::Value>` and `PartialConfig.backends: Option<BTreeMap<String, serde_yaml::Value>>`. Task 5 reads `config.backends.get("kokoro")`.

- [ ] **Step 1: Write the failing test**

Append to `crates/teleprompt-core/tests/config.rs`:

```rust
#[test]
fn backend_settings_survive_the_merge_without_core_understanding_them() {
    let base: PartialConfig = PartialConfig::from_yaml(
        "backends:\n  kokoro:\n    base_url: \"http://localhost:8880\"\n    timeout_ms: 30000\n",
    )
    .unwrap();
    let over: PartialConfig =
        PartialConfig::from_yaml("backends:\n  kokoro:\n    timeout_ms: 5000\n").unwrap();

    let merged = Config::merged(&[base, over]);
    let k = merged.backends.get("kokoro").expect("kokoro settings kept");

    // Per-key merge, not whole-table replacement: the later layer overrides
    // `timeout_ms` and leaves `base_url` alone. Whole-table replacement would
    // mean a script overriding one setting silently discards the project's
    // other ones.
    assert_eq!(k.get("timeout_ms").unwrap().as_u64(), Some(5000));
    assert_eq!(
        k.get("base_url").unwrap().as_str(),
        Some("http://localhost:8880")
    );
}

#[test]
fn an_unknown_backend_table_is_not_an_error() {
    // Core does not validate backend ids. A config naming a backend this
    // build does not ship must parse; `voice.backend` selection is where an
    // unknown id is reported, with the available list.
    let c = PartialConfig::from_yaml("backends:\n  elevenlabs:\n    profile: jan\n").unwrap();
    let merged = Config::merged(&[c]);
    assert!(merged.backends.contains_key("elevenlabs"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p teleprompt-core --test config backend_settings_survive`
Expected: FAIL — `no field 'backends' on type 'Config'`.

- [ ] **Step 3: Add the field to both config types**

In `crates/teleprompt-core/src/config.rs`, add to `Config` (after `default_scene`):

```rust
    /// Backend-native settings, keyed by backend id, exactly as written
    /// under `backends:` in config or front matter.
    ///
    /// Core never interprets these. It cannot: the whole point of the
    /// pluggable contract is that teleprompt does not know what backends
    /// exist, so a named field per backend here would be a hardcoded list
    /// wearing a config's clothes. Each backend deserializes its own slice
    /// by its own id, and an id this build does not ship is not an error at
    /// this layer — `voice.backend` selection reports that, with the
    /// available list.
    pub backends: BTreeMap<String, serde_yaml::Value>,
```

Add to `PartialConfig` (the struct has `#[serde(deny_unknown_fields)]`, so the key must be declared to parse at all):

```rust
    pub backends: Option<BTreeMap<String, serde_yaml::Value>>,
```

Update `Config::default()` to include `backends: BTreeMap::new()`. If `Config` uses `#[derive(Default)]`, no change is needed.

- [ ] **Step 4: Merge per key, not per table**

In `Config::merged`, after the `if let Some(scenes) = &layer.scene { ... }` block:

```rust
            if let Some(bs) = &layer.backends {
                for (id, settings) in bs {
                    // Merge the inner mapping key by key so a script
                    // overriding one setting does not discard the project's
                    // others. A non-mapping value replaces wholesale —
                    // there is nothing sensible to merge into.
                    match (c.backends.get_mut(id), settings.as_mapping()) {
                        (Some(existing), Some(new)) => {
                            if let Some(target) = existing.as_mapping_mut() {
                                for (k, v) in new {
                                    target.insert(k.clone(), v.clone());
                                }
                                continue;
                            }
                            c.backends.insert(id.clone(), settings.clone());
                        }
                        _ => {
                            c.backends.insert(id.clone(), settings.clone());
                        }
                    }
                }
            }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p teleprompt-core --test config`
Expected: PASS, including the two new tests.

- [ ] **Step 6: Run the full suite for snapshot fallout**

Run: `cargo test --workspace`
Expected: PASS. `Config` gained a field; if any snapshot serializes `Config` wholesale it will need `cargo insta accept` — inspect the diff and confirm only `backends: {}` appeared before accepting.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-core/src/config.rs crates/teleprompt-core/tests/config.rs
git commit -m "feat(core): carry backend-native settings without naming a backend"
```

---

## Task 2: The crate, its config, and its cache identity

The crate skeleton and `KokoroConfig`, including the version string that keys the cache. No HTTP yet — this task is about getting the identity right, because getting it wrong is the failure the contract doc calls out by name.

**Files:**
- Create: `crates/teleprompt-voice-kokoro/Cargo.toml`, `src/lib.rs`, `src/config.rs`
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`)
- Test: `crates/teleprompt-voice-kokoro/tests/config.rs`

**Interfaces:**
- Consumes: `Config.backends` from Task 1 (as a `serde_yaml::Value`, not the `Config` type — this crate must not depend on `teleprompt-core`).
- Produces:
  - `KokoroConfig { base_url: String, timeout_ms: u64, concurrency: usize, model: String }`
  - `KokoroConfig::default()` — `base_url: "http://localhost:8880"`, `timeout_ms: 30000`, `concurrency: 4`, `model: "kokoro"`
  - `KokoroConfig::from_value(v: &serde_yaml::Value) -> Result<KokoroConfig, String>`
  - `KokoroConfig::version_string(&self) -> String`
  - `pub const KOKORO_SAMPLE_RATE: u32 = 24_000;`

- [ ] **Step 1: Create the crate manifest**

`crates/teleprompt-voice-kokoro/Cargo.toml`:

```toml
[package]
name = "teleprompt-voice-kokoro"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
teleprompt-voice = { path = "../teleprompt-voice" }
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt", "net", "io-util"] }
```

There is deliberately no `teleprompt-core` dependency. `teleprompt-voice-null` had one, never used it, and it was removed precisely because it weakened the proof that a backend needs only the contract. Do not add one here; if something seems to require it, that is a finding about the contract.

Add to the workspace `[workspace.dependencies]` in the root `Cargo.toml`, keeping the list alphabetical:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

`default-features = false` drops the OpenSSL path and the default `native-tls` stack. `rustls-tls` is kept rather than dropped entirely: Kokoro is usually local plaintext, but pointing `base_url` at an HTTPS host is a reasonable thing to do and a TLS-less client would fail it with a confusing error.

- [ ] **Step 2: Write the failing test**

`crates/teleprompt-voice-kokoro/tests/config.rs`:

```rust
use teleprompt_voice_kokoro::KokoroConfig;

fn yaml(s: &str) -> serde_yaml::Value {
    serde_yaml::from_str(s).unwrap()
}

#[test]
fn defaults_match_the_spec() {
    let c = KokoroConfig::default();
    assert_eq!(c.base_url, "http://localhost:8880");
    assert_eq!(c.timeout_ms, 30_000);
    assert_eq!(c.concurrency, 4);
    assert_eq!(c.model, "kokoro");
}

#[test]
fn partial_settings_keep_the_other_defaults() {
    let c = KokoroConfig::from_value(&yaml("timeout_ms: 5000")).unwrap();
    assert_eq!(c.timeout_ms, 5_000);
    assert_eq!(c.base_url, "http://localhost:8880");
    assert_eq!(c.concurrency, 4);
}

#[test]
fn a_trailing_slash_on_base_url_does_not_produce_a_double_slash() {
    let c = KokoroConfig::from_value(&yaml("base_url: \"http://x:8880/\"")).unwrap();
    assert_eq!(c.base_url, "http://x:8880");
}

#[test]
fn an_unknown_setting_is_rejected_rather_than_ignored() {
    // A typo in a backend setting is otherwise invisible: the run succeeds
    // with the default and the author never learns their value did nothing.
    let err = KokoroConfig::from_value(&yaml("timeuot_ms: 5000")).unwrap_err();
    assert!(err.contains("timeuot_ms"), "{err}");
}

#[test]
fn zero_concurrency_is_rejected() {
    let err = KokoroConfig::from_value(&yaml("concurrency: 0")).unwrap_err();
    assert!(err.contains("concurrency"), "{err}");
}

// The load-bearing one. `teleprompt_cache::key` is built from `backend_id`,
// `backend_version` and the SynthRequest — text, locale, voice, speed. A
// Kokoro server's audio also depends on which server it is and which model
// it loaded, and neither reaches the key except through this string.
#[test]
fn the_version_string_separates_hosts_and_models() {
    let a = KokoroConfig::from_value(&yaml("base_url: \"http://localhost:8880\"")).unwrap();
    let b = KokoroConfig::from_value(&yaml("base_url: \"http://gpu-box:8880\"")).unwrap();
    let c = KokoroConfig::from_value(&yaml(
        "base_url: \"http://localhost:8880\"\nmodel: kokoro-v1_1",
    ))
    .unwrap();

    assert_ne!(a.version_string(), b.version_string(), "host must be in the key");
    assert_ne!(a.version_string(), c.version_string(), "model must be in the key");
}

#[test]
fn settings_that_cannot_change_the_audio_stay_out_of_the_version() {
    // Otherwise every timeout or concurrency retune invalidates the whole
    // cache and re-synthesizes a project that did not change.
    let a = KokoroConfig::from_value(&yaml("timeout_ms: 30000\nconcurrency: 4")).unwrap();
    let b = KokoroConfig::from_value(&yaml("timeout_ms: 1000\nconcurrency: 16")).unwrap();
    assert_eq!(a.version_string(), b.version_string());
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p teleprompt-voice-kokoro --test config`
Expected: FAIL — the crate does not exist yet.

- [ ] **Step 4: Write `src/config.rs`**

```rust
use serde::Deserialize;

/// Kokoro-FastAPI emits 24 kHz mono. `Pcm` carries the rate, the WAV
/// encoder writes any rate, and the manifest reports whatever the backend
/// produced — so this constant is the backend's own fact, not a global one.
pub const KOKORO_SAMPLE_RATE: u32 = 24_000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct KokoroConfig {
    pub base_url: String,
    pub timeout_ms: u64,
    /// Bounded fan-out for `dub`. A local model server is the bottleneck and
    /// unbounded concurrency makes it slower, not faster.
    pub concurrency: usize,
    /// Sent as the request's `model` field and folded into the cache key.
    pub model: String,
}

impl Default for KokoroConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8880".to_string(),
            timeout_ms: 30_000,
            concurrency: 4,
            model: "kokoro".to_string(),
        }
    }
}

impl KokoroConfig {
    pub fn from_value(v: &serde_yaml::Value) -> Result<Self, String> {
        let mut c: KokoroConfig = serde_yaml::from_value(v.clone())
            .map_err(|e| format!("invalid `backends.kokoro` settings: {e}"))?;
        while c.base_url.ends_with('/') {
            c.base_url.pop();
        }
        if c.base_url.is_empty() {
            return Err("`backends.kokoro.base_url` must not be empty".to_string());
        }
        if c.concurrency == 0 {
            return Err("`backends.kokoro.concurrency` must be at least 1".to_string());
        }
        Ok(c)
    }

    /// What `VoiceCapabilities::version` returns, and therefore part of every
    /// cache key this backend's audio is stored under.
    ///
    /// Host and model are both in it because both change the audio while
    /// leaving the `SynthRequest` identical: two servers with different
    /// checkpoints answer the same request differently, and nothing above
    /// the cache could tell. The contract doc on
    /// `VoiceCapabilities::version` names this exact hazard.
    ///
    /// The cost is a real one and worth stating: `localhost` and `127.0.0.1`
    /// are different strings, so pointing the same server at a different
    /// name re-synthesizes the project once. That is the right trade —
    /// a spurious miss is slow and visible, whereas a spurious hit serves
    /// one voice's audio under another's name, permanently.
    pub fn version_string(&self) -> String {
        let host = self
            .base_url
            .split("://")
            .nth(1)
            .unwrap_or(&self.base_url);
        format!("{}@{}", self.model, host)
    }
}
```

- [ ] **Step 5: Write `src/lib.rs`**

```rust
//! The Kokoro-FastAPI voice backend.
//!
//! teleprompt speaks HTTP to a server the author runs and owns no Python.
//! Like `teleprompt-voice-null`, this crate is written against
//! `teleprompt-voice`'s public API alone — it does not depend on
//! `teleprompt-core`. That is the standing proof that the contract admits a
//! backend nothing in it was designed around.

mod config;

pub use config::{KokoroConfig, KOKORO_SAMPLE_RATE};
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p teleprompt-voice-kokoro --test config`
Expected: PASS, all seven.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-voice-kokoro Cargo.toml Cargo.lock
git commit -m "feat(kokoro): the crate, its settings, and its cache identity"
```

---

## Task 3: The stub server and a real synthesis round trip

The HTTP call and the PCM decode, tested against an in-process server. This is the heart of the crate.

**Files:**
- Create: `crates/teleprompt-voice-kokoro/src/client.rs`, `src/backend.rs`, `tests/stub/mod.rs`, `tests/synth.rs`
- Modify: `crates/teleprompt-voice-kokoro/src/lib.rs`

**Interfaces:**
- Consumes: `KokoroConfig`, `KOKORO_SAMPLE_RATE` from Task 2. From `teleprompt_voice`: `VoiceBackend`, `SynthRequest`, `Synthesized`, `Pcm`, `VoiceError`, `VoiceCapabilities`, `LanguageSupport`, `async_trait` (re-exported by `teleprompt-voice`).
- Produces: `KokoroVoice::new(config: KokoroConfig) -> Result<KokoroVoice, String>` and its `VoiceBackend` impl (`id() -> "kokoro"`).

- [ ] **Step 1: Write the stub server**

`crates/teleprompt-voice-kokoro/tests/stub/mod.rs`:

```rust
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// What the stub should do for the next request.
#[derive(Clone)]
pub enum Reply {
    /// 200 with these bytes as the body.
    Ok(Vec<u8>),
    /// 200, but claim a longer body than is sent, then close. Exercises the
    /// truncated-response path.
    Truncated { body: Vec<u8>, claim: usize },
    /// A non-200 with this body.
    Status(u16, String),
    /// Accept the connection and never answer. Exercises the timeout path.
    Hang,
}

pub struct Stub {
    pub base_url: String,
    pub requests: Arc<Mutex<Vec<String>>>,
}

/// Spawns a one-route HTTP/1.1 server on an ephemeral port. Handles requests
/// until dropped. Returns the URL to point `base_url` at.
pub async fn spawn(reply: Reply) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let reply = reply.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let raw = String::from_utf8_lossy(&buf[..n]).to_string();
                seen.lock().unwrap().push(raw);

                match reply {
                    Reply::Hang => {
                        // Hold the connection open with no response.
                        futures_hang(&mut socket).await;
                    }
                    Reply::Ok(body) => {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: \
                             application/octet-stream\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = socket.write_all(head.as_bytes()).await;
                        let _ = socket.write_all(&body).await;
                    }
                    Reply::Truncated { body, claim } => {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {claim}\r\nConnection: \
                             close\r\n\r\n"
                        );
                        let _ = socket.write_all(head.as_bytes()).await;
                        let _ = socket.write_all(&body).await;
                        // Close without sending the rest.
                    }
                    Reply::Status(code, body) => {
                        let head = format!(
                            "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: \
                             close\r\n\r\n",
                            body.len()
                        );
                        let _ = socket.write_all(head.as_bytes()).await;
                        let _ = socket.write_all(body.as_bytes()).await;
                    }
                }
                let _ = socket.shutdown().await;
            });
        }
    });

    Stub {
        base_url: format!("http://{addr}"),
        requests,
    }
}

async fn futures_hang(socket: &mut tokio::net::TcpStream) {
    let mut sink = [0u8; 1];
    // Reading blocks until the peer gives up; that is the hang we want.
    let _ = socket.read(&mut sink).await;
}

impl Stub {
    /// The JSON body of the first request, parsed.
    pub fn first_body(&self) -> serde_json::Value {
        let raw = self.requests.lock().unwrap()[0].clone();
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("body was not JSON: {e}\n{raw}"))
    }

    pub fn first_request_line(&self) -> String {
        self.requests.lock().unwrap()[0]
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    }
}
```

- [ ] **Step 2: Write the failing synthesis tests**

`crates/teleprompt-voice-kokoro/tests/synth.rs`:

```rust
mod stub;

use stub::{spawn, Reply};
use teleprompt_voice::{SynthRequest, VoiceBackend, VoiceError};
use teleprompt_voice_kokoro::{KokoroConfig, KokoroVoice, KOKORO_SAMPLE_RATE};

fn req(text: &str) -> SynthRequest {
    SynthRequest {
        text: text.to_string(),
        locale: "en".to_string(),
        voice: Some("af_heart".to_string()),
        speed: 1.0,
    }
}

fn backend(base_url: &str, timeout_ms: u64) -> KokoroVoice {
    KokoroVoice::new(KokoroConfig {
        base_url: base_url.to_string(),
        timeout_ms,
        ..KokoroConfig::default()
    })
    .unwrap()
}

/// 2400 little-endian i16 samples = 4800 bytes = exactly 100 ms at 24 kHz.
fn pcm_bytes(samples: usize) -> Vec<u8> {
    (0..samples).flat_map(|i| ((i % 1000) as i16).to_le_bytes()).collect()
}

#[tokio::test]
async fn a_successful_synthesis_decodes_to_24khz_mono() {
    let s = spawn(Reply::Ok(pcm_bytes(2400))).await;
    let out = backend(&s.base_url, 30_000).synthesize(&req("hello")).await.unwrap();

    assert_eq!(out.pcm.sample_rate, KOKORO_SAMPLE_RATE);
    assert_eq!(out.pcm.channels, 1);
    assert_eq!(out.pcm.samples.len(), 2400);
    assert_eq!(out.pcm.duration_ms(), 100);
    // Spec §7: word timings only exist on a /dev/ path, which is not a
    // stable interface to publish a manifest field from.
    assert!(out.word_timings.is_none());
}

#[tokio::test]
async fn the_request_body_is_what_the_spec_says() {
    let s = spawn(Reply::Ok(pcm_bytes(2))).await;
    let r = SynthRequest {
        text: "hello".to_string(),
        locale: "en".to_string(),
        voice: Some("af_bella".to_string()),
        speed: 1.25,
    };
    backend(&s.base_url, 30_000).synthesize(&r).await.unwrap();

    assert_eq!(s.first_request_line(), "POST /v1/audio/speech HTTP/1.1");
    let body = s.first_body();
    assert_eq!(body["model"], "kokoro");
    assert_eq!(body["input"], "hello");
    assert_eq!(body["voice"], "af_bella");
    assert_eq!(body["response_format"], "pcm");
    // Sent to the server so the voice is *generated* at this rate rather
    // than resampled — core spec §6.2's boundary.
    assert_eq!(body["speed"], 1.25);
}

#[tokio::test]
async fn an_odd_byte_count_is_an_error_not_a_dropped_sample() {
    // i16 samples are byte pairs. A trailing odd byte means the body is not
    // what it claims; silently dropping it would publish a duration one
    // sample short of the audio and hide a real protocol problem.
    let s = spawn(Reply::Ok(vec![1, 2, 3])).await;
    let err = backend(&s.base_url, 30_000).synthesize(&req("x")).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("odd") || msg.contains("3 bytes"), "{msg}");
}

#[tokio::test]
async fn an_empty_body_is_an_error() {
    // Zero samples would be a zero-length WAV published as a real segment.
    let s = spawn(Reply::Ok(Vec::new())).await;
    let err = backend(&s.base_url, 30_000).synthesize(&req("x")).await.unwrap_err();
    assert!(err.to_string().contains("empty"), "{err}");
}

#[tokio::test]
async fn a_non_200_names_the_url_and_the_status() {
    let s = spawn(Reply::Status(500, "boom".to_string())).await;
    let err = backend(&s.base_url, 30_000).synthesize(&req("x")).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("500"), "{msg}");
    assert!(msg.contains(&s.base_url), "{msg}");
    // Spec §7.1: never a silent fallback to silence.
    assert!(!matches!(err, VoiceError::Unsupported { .. }));
}

#[tokio::test]
async fn a_timeout_names_the_url_and_the_limit() {
    let s = spawn(Reply::Hang).await;
    let err = backend(&s.base_url, 150).synthesize(&req("x")).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("150"), "{msg}");
    assert!(msg.contains(&s.base_url), "{msg}");
}

#[tokio::test]
async fn a_truncated_body_is_an_error_not_short_audio() {
    let s = spawn(Reply::Truncated { body: pcm_bytes(10), claim: 4800 }).await;
    let err = backend(&s.base_url, 30_000).synthesize(&req("x")).await.unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[tokio::test]
async fn an_unreachable_server_names_the_url() {
    // Port 1 on loopback: nothing listens, connection refused immediately.
    let err = backend("http://127.0.0.1:1", 30_000).synthesize(&req("x")).await.unwrap_err();
    assert!(err.to_string().contains("127.0.0.1:1"), "{err}");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p teleprompt-voice-kokoro --test synth`
Expected: FAIL — `KokoroVoice` does not exist.

- [ ] **Step 4: Write `src/client.rs`**

```rust
use teleprompt_voice::{Pcm, VoiceError};

use crate::config::{KokoroConfig, KOKORO_SAMPLE_RATE};

pub struct Client {
    http: reqwest::Client,
    cfg: KokoroConfig,
}

impl Client {
    pub fn new(cfg: KokoroConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(cfg.timeout_ms))
            .build()
            .map_err(|e| format!("cannot build HTTP client: {e}"))?;
        Ok(Self { http, cfg })
    }

    pub fn config(&self) -> &KokoroConfig {
        &self.cfg
    }

    /// Every failure here is fatal to the command. Spec §7.1: a broken
    /// synthesizer is not a voice tier, so there is no fallback to silence
    /// anywhere in this file.
    fn fail(&self, what: &str) -> VoiceError {
        VoiceError::Other(format!("kokoro at {}: {what}", self.cfg.base_url))
    }

    pub async fn speech(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: f64,
    ) -> Result<Pcm, VoiceError> {
        let url = format!("{}/v1/audio/speech", self.cfg.base_url);
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "input": text,
            "response_format": "pcm",
            "speed": speed,
        });
        if let Some(v) = voice {
            body["voice"] = serde_json::Value::String(v.to_string());
        }

        let resp = self.http.post(&url).json(&body).send().await.map_err(|e| {
            if e.is_timeout() {
                self.fail(&format!("no response within {}ms", self.cfg.timeout_ms))
            } else {
                self.fail(&format!("request failed: {e}"))
            }
        })?;

        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            let detail = detail.trim();
            let tail = if detail.is_empty() {
                String::new()
            } else {
                format!(" — {}", truncate(detail, 200))
            };
            return Err(self.fail(&format!("returned {}{tail}", status.as_u16())));
        }

        let bytes = resp.bytes().await.map_err(|e| {
            if e.is_timeout() {
                self.fail(&format!("no response within {}ms", self.cfg.timeout_ms))
            } else {
                self.fail(&format!("response body incomplete: {e}"))
            }
        })?;

        decode_pcm(&bytes).map_err(|what| self.fail(&what))
    }

    pub async fn voices(&self) -> Result<Vec<String>, VoiceError> {
        let url = format!("{}/v1/audio/voices", self.cfg.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| self.fail(&format!("cannot list voices: {e}")))?;
        if !resp.status().is_success() {
            return Err(self.fail(&format!(
                "listing voices returned {}",
                resp.status().as_u16()
            )));
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| self.fail(&format!("voice list was not JSON: {e}")))?;

        // Kokoro-FastAPI answers `{"voices": [...]}`; some OpenAI-compatible
        // servers answer a bare array. Accept both rather than making the
        // author debug a shape mismatch.
        let arr = v
            .get("voices")
            .and_then(|x| x.as_array())
            .or_else(|| v.as_array())
            .ok_or_else(|| self.fail("voice list had no `voices` array"))?;

        Ok(arr
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect())
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n).collect::<String>() + "…"
}

/// `response_format: "pcm"` is raw little-endian 16-bit mono at 24 kHz. No
/// mp3 or wav decoder enters the workspace.
///
/// Both rejections below matter: an odd byte count means the body is not
/// what it claims, and an empty body would become a zero-length WAV
/// published as a real segment. Neither is recoverable by guessing.
fn decode_pcm(bytes: &[u8]) -> Result<Pcm, String> {
    if bytes.is_empty() {
        return Err("returned an empty audio body".to_string());
    }
    if bytes.len() % 2 != 0 {
        return Err(format!(
            "returned {} bytes, an odd count for 16-bit samples",
            bytes.len()
        ));
    }
    let samples = bytes
        .chunks_exact(2)
        .map(|p| i16::from_le_bytes([p[0], p[1]]))
        .collect();
    Ok(Pcm {
        sample_rate: KOKORO_SAMPLE_RATE,
        channels: 1,
        samples,
    })
}
```

- [ ] **Step 5: Write `src/backend.rs`**

```rust
use teleprompt_voice::{
    async_trait, LanguageSupport, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities,
    VoiceError,
};

use crate::client::Client;
use crate::config::KokoroConfig;

pub struct KokoroVoice {
    client: Client,
    version: String,
}

impl KokoroVoice {
    pub fn new(cfg: KokoroConfig) -> Result<Self, String> {
        let version = cfg.version_string();
        Ok(Self {
            client: Client::new(cfg)?,
            version,
        })
    }

    /// The voices the server reports. Used by `doctor` and by `dub`'s
    /// one-shot validation — never by `check`, which must not touch the
    /// network.
    pub async fn voices(&self) -> Result<Vec<String>, VoiceError> {
        self.client.voices().await
    }

    pub fn base_url(&self) -> &str {
        &self.client.config().base_url
    }

    pub fn concurrency(&self) -> usize {
        self.client.config().concurrency
    }
}

#[async_trait]
impl VoiceBackend for KokoroVoice {
    fn id(&self) -> &str {
        "kokoro"
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            // Kokoro's voices are language-specific and the server is the
            // authority on which exist; enumerating a fixed list here would
            // go stale against a server we do not ship.
            languages: LanguageSupport::Any,
            cloning: false,
            cross_lingual: false,
            // Available only on POST /dev/captioned_speech. A `/dev/` path
            // is not a stable interface to build a published manifest field
            // on. Revisit when it stabilises; the manifest already omits
            // `words` when absent.
            word_timings: false,
            ssml: false,
            speed_control: true,
            version: self.version.clone(),
        }
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        let pcm = self
            .client
            .speech(&req.text, req.voice.as_deref(), req.speed)
            .await?;
        Ok(Synthesized {
            pcm,
            word_timings: None,
        })
    }
}
```

- [ ] **Step 6: Wire the modules in `src/lib.rs`**

Replace the body of `crates/teleprompt-voice-kokoro/src/lib.rs`'s module section with:

```rust
mod backend;
mod client;
mod config;

pub use backend::KokoroVoice;
pub use config::{KokoroConfig, KOKORO_SAMPLE_RATE};
```

- [ ] **Step 7: Confirm `async_trait` is re-exported**

Run: `grep -n "pub use async_trait" crates/teleprompt-voice/src/lib.rs`
Expected: one hit. Delivery A added it so a third-party backend does not take the dependency itself. If it is missing, that is a finding — add `pub use async_trait::async_trait;` to `teleprompt-voice`'s `lib.rs` rather than adding `async-trait` to this crate's `Cargo.toml`.

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p teleprompt-voice-kokoro`
Expected: PASS, all config and synth tests.

- [ ] **Step 9: Commit**

```bash
git add crates/teleprompt-voice-kokoro Cargo.lock
git commit -m "feat(kokoro): synthesis over HTTP, with a stub server to prove it"
```

---

## Task 4: Registration, and selection that reads its own settings

The one-line claim. Registering Kokoro means constructing it from config, which means the CLI must hand each backend its own settings slice.

**Files:**
- Modify: `crates/teleprompt-cli/Cargo.toml`, `crates/teleprompt-cli/src/voice.rs`
- Test: `crates/teleprompt-cli/tests/voice_selection.rs` (create)

**Interfaces:**
- Consumes: `Config.backends` (Task 1), `KokoroVoice::new` / `KokoroConfig::from_value` (Tasks 2–3).
- Produces: `pub fn registry_for(backends: &BTreeMap<String, serde_yaml::Value>) -> Result<VoiceRegistry, String>`. Existing `registry()` becomes `registry_for(&BTreeMap::new())`, so every current caller keeps working with defaults.

- [ ] **Step 1: Write the failing test**

`crates/teleprompt-cli/tests/voice_selection.rs`:

```rust
use std::collections::BTreeMap;

use teleprompt_cli::voice::{registry, registry_for};

fn settings(yaml: &str) -> BTreeMap<String, serde_yaml::Value> {
    let mut m = BTreeMap::new();
    m.insert("kokoro".to_string(), serde_yaml::from_str(yaml).unwrap());
    m
}

#[test]
fn kokoro_is_registered_by_default() {
    let r = registry();
    assert_eq!(r.available(), vec!["kokoro", "null"]);
}

#[test]
fn kokoro_takes_its_settings_from_the_backends_map() {
    let r = registry_for(&settings("base_url: \"http://gpu-box:8880\"")).unwrap();
    let k = r.get("kokoro").unwrap();
    // The version string is what keys the cache, so this is the observable
    // that proves the settings reached the backend rather than the default.
    assert!(
        k.capabilities().version.contains("gpu-box"),
        "{}",
        k.capabilities().version
    );
}

#[test]
fn a_bad_backend_setting_is_reported_not_swallowed() {
    let err = registry_for(&settings("concurrency: 0")).unwrap_err();
    assert!(err.contains("concurrency"), "{err}");
}

#[test]
fn settings_for_a_backend_this_build_lacks_are_ignored_here() {
    // Reporting an unknown backend is `resolve`'s job, and only when the
    // script actually selects it. A project carrying settings for a backend
    // it does not currently use must still build.
    let mut m = BTreeMap::new();
    m.insert(
        "elevenlabs".to_string(),
        serde_yaml::from_str("profile: jan").unwrap(),
    );
    assert!(registry_for(&m).is_ok());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p teleprompt-cli --test voice_selection`
Expected: FAIL — `registry_for` is not defined.

- [ ] **Step 3: Add the dependency**

In `crates/teleprompt-cli/Cargo.toml`, under `[dependencies]`:

```toml
teleprompt-voice-kokoro = { path = "../teleprompt-voice-kokoro" }
serde_yaml = { workspace = true }
```

(`serde_yaml` may already be present — check before adding a duplicate key.)

- [ ] **Step 4: Rewrite `crates/teleprompt-cli/src/voice.rs`**

```rust
use std::collections::BTreeMap;
use std::sync::Arc;

use teleprompt_voice::{VoiceBackend, VoiceRegistry};
use teleprompt_voice_kokoro::{KokoroConfig, KokoroVoice};
use teleprompt_voice_null::NullVoice;

/// The backends this build ships, each constructed from its own slice of
/// `backends:`. Adding one is a line here plus a crate — nothing else in the
/// workspace changes, and in particular `teleprompt-core` never learns the
/// new backend's name.
pub fn registry_for(
    backends: &BTreeMap<String, serde_yaml::Value>,
) -> Result<VoiceRegistry, String> {
    let mut r = VoiceRegistry::default();
    r.register(Arc::new(NullVoice::default()));

    let kokoro = match backends.get("kokoro") {
        Some(v) => KokoroConfig::from_value(v)?,
        None => KokoroConfig::default(),
    };
    r.register(Arc::new(KokoroVoice::new(kokoro)?));

    Ok(r)
}

/// The default registry, for callers with no project config in hand
/// (`doctor` outside a project, tests). Settings-free, so every backend gets
/// its defaults.
pub fn registry() -> VoiceRegistry {
    registry_for(&BTreeMap::new()).expect("default backend settings are always valid")
}

pub fn resolve(registry: &VoiceRegistry, id: &str) -> Result<Arc<dyn VoiceBackend>, String> {
    registry.get(id).ok_or_else(|| {
        format!(
            "unknown voice backend `{id}` (available: {})",
            registry.available().join(", ")
        )
    })
}
```

- [ ] **Step 5: Route project config into the registry**

`compile_script` and `run_dub` currently call `crate::voice::registry()`, which discards the project's `backends:`. Find each call site:

Run: `grep -rn "voice::registry()" crates/teleprompt-cli/src`

For each, the project's config is already resolved nearby. Replace the call with `crate::voice::registry_for(&config.backends)?`, mapping the error into that function's existing error type (a `Vec<String>` for `compile_script`, `DubError::Validation(vec![e])` for `run_dub`). Where the config is not yet available at that point — because the registry is built before the script is read — keep `registry()` and leave a comment saying the project's settings do not reach it, rather than restructuring; Task 5 needs the same plumbing and will finish it.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p teleprompt-cli --test voice_selection && cargo test --workspace`
Expected: PASS. `doctor`'s `voice_backends` list now reads `kokoro, null`, so its snapshot or assertion changes — update it and confirm the only change is the added backend.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-cli Cargo.lock
git commit -m "feat(cli): register kokoro, and hand each backend its own settings"
```

---

## Task 5: `dub` validates the voice once, before synthesizing anything

Spec §7 (as amended): the voice list is checked at the start of `dub`, not at `check` time. An unknown voice must fail before the first segment, not after the twentieth.

**Files:**
- Modify: `crates/teleprompt-cli/src/cmd/dub.rs`
- Test: `crates/teleprompt-cli/tests/dub.rs`

**Interfaces:**
- Consumes: `KokoroVoice::voices()` (Task 3), `registry_for` (Task 4).
- Produces: no new public API. Behaviour: `run_dub` returns `DubError::Validation` naming the configured voice and listing available ones.

- [ ] **Step 1: Write the failing test**

Add to `crates/teleprompt-cli/tests/dub.rs`. Use the existing test-project helpers in that file; the point is that with the `null` backend nothing probes the network at all.

```rust
#[tokio::test]
async fn dub_with_the_null_backend_makes_no_network_call() {
    // The guard added for kokoro must not become a probe every backend
    // pays for. `null` has no server, so a probe here would either fail or
    // hang — and this test would catch it.
    let p = project_with_script("Hello there.\n");
    let out = run_dub(&p.project, &p.script, "en", &p.out, false).await;
    assert!(out.is_ok(), "{:?}", out.err());
}
```

- [ ] **Step 2: Run test to verify it passes already**

Run: `cargo test -p teleprompt-cli --test dub dub_with_the_null_backend`
Expected: PASS. This one is a regression guard for Step 4, not a driver — it must keep passing after the validation lands. State that in the report rather than presenting it as new coverage.

- [ ] **Step 3: Write the failing test that does drive the change**

```rust
#[tokio::test]
async fn an_unknown_kokoro_voice_fails_before_any_segment_is_synthesized() {
    let stub = kokoro_stub_listing(&["af_heart", "af_bella"]).await;
    let p = project_with_config_and_script(
        &format!(
            "voice:\n  backend: kokoro\n  voice: nonexistent\nbackends:\n  kokoro:\n    \
             base_url: \"{}\"\n",
            stub.base_url
        ),
        "Hello there.\n",
    );

    let err = run_dub(&p.project, &p.script, "en", &p.out, false)
        .await
        .unwrap_err();
    let DubError::Validation(msgs) = err else {
        panic!("expected a validation error, got {err:?}");
    };
    let joined = msgs.join("\n");
    assert!(joined.contains("nonexistent"), "{joined}");
    assert!(joined.contains("af_heart"), "must list what is available: {joined}");

    // Nothing was synthesized: the stub saw the voice list request and
    // nothing else.
    assert_eq!(stub.request_count(), 1, "must fail before the first segment");
}
```

Add `kokoro_stub_listing` to the same test file — a minimal `/v1/audio/voices` responder built the same way as `crates/teleprompt-voice-kokoro/tests/stub/mod.rs`. Do not import that module across crates; copy the twenty lines needed for a JSON GET responder, and say in the report that the duplication is deliberate (a test helper crate for two call sites is not worth a crate).

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test -p teleprompt-cli --test dub an_unknown_kokoro_voice`
Expected: FAIL — `dub` synthesizes and the count is 2, or the error is a `Runtime` rather than a `Validation`.

- [ ] **Step 5: Add the one-shot validation**

In `run_dub_with`, immediately after `compile_script_with` returns `(compiled, backend)` and before the render loop:

```rust
    // Spec §7: the voice list is checked once here, not at `check` time.
    // `check` must stay offline and synchronous, and a gate that only works
    // when a server happens to be running is worse than no gate — the same
    // script would pass on one machine and fail on another.
    //
    // Downcasting rather than widening the trait: "list your voices" is not
    // something every backend can do, and adding an
    // `Option<Vec<String>>`-returning method to a single-method contract to
    // serve one implementation is exactly the speculative surface the
    // contract was sharpened to remove.
    if let Some(kokoro) = backend.as_any().downcast_ref::<KokoroVoice>() {
        let wanted = compiled
            .narration
            .iter()
            .filter_map(|d| d.synth_request.voice.clone())
            .collect::<std::collections::BTreeSet<_>>();
        if !wanted.is_empty() {
            let available = kokoro
                .voices()
                .await
                .map_err(|e| DubError::Validation(vec![e.to_string()]))?;
            let mut problems = Vec::new();
            for v in &wanted {
                if !available.contains(v) {
                    problems.push(format!(
                        "voice `{v}` is not available on the kokoro server at {} \
                         (available: {})",
                        kokoro.base_url(),
                        available.join(", ")
                    ));
                }
            }
            if !problems.is_empty() {
                return Err(DubError::Validation(problems));
            }
        }
    }
```

This needs `VoiceBackend` to expose `as_any`. Add to the trait in `crates/teleprompt-voice/src/contract.rs`:

```rust
    /// Escape hatch for capabilities that are genuinely one backend's own.
    ///
    /// The contract stays one method because that is what makes it
    /// implementable; a backend that can do something extra — list its
    /// server's voices, report a queue depth — exposes it as an inherent
    /// method, and a caller that knows the concrete type reaches it here.
    /// Nothing on the `check`/`plan`/`diff` path uses this, and nothing
    /// should: downcasting from the inner loop would be a way to smuggle a
    /// network call into it.
    fn as_any(&self) -> &dyn std::any::Any;
```

Implement it in `NullVoice`, `KokoroVoice`, and both test stubs (`crates/teleprompt-voice/tests/stub/mod.rs` and any `impl VoiceBackend` in `crates/teleprompt-compile/tests/`) as:

```rust
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
```

Run `grep -rn "impl VoiceBackend" crates/` first and fix every hit; a missed one is a compile error, not a silent gap.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p teleprompt-cli --test dub && cargo test --workspace`
Expected: PASS, including the null-backend no-probe guard from Step 1.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-voice crates/teleprompt-voice-null crates/teleprompt-voice-kokoro crates/teleprompt-compile crates/teleprompt-cli
git commit -m "feat(dub): reject an unknown voice before synthesizing the first segment"
```

---

## Task 6: `doctor` probes the configured backend

**Files:**
- Modify: `crates/teleprompt-cli/src/cmd/doctor.rs`, `crates/teleprompt-cli/src/main.rs`
- Test: `crates/teleprompt-cli/tests/doctor.rs`

**Interfaces:**
- Consumes: `KokoroVoice::voices()`, `KokoroVoice::base_url()`.
- Produces: `pub async fn doctor_report(registry: &SceneRegistry) -> DoctorReport`, with `DoctorReport.voice_probe: Option<String>`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn doctor_reports_a_reachable_server_with_its_voice_count() {
    let stub = kokoro_stub_listing(&["af_heart", "af_bella", "am_adam"]).await;
    let report = doctor_report_for(&stub.base_url).await;
    let line = report.voice_probe.expect("probe ran");
    assert!(line.contains(&stub.base_url), "{line}");
    assert!(line.contains("reachable"), "{line}");
    assert!(line.contains('3'), "must report the voice count: {line}");
    assert!(report.ok, "a reachable server is not a problem");
}

#[tokio::test]
async fn an_unreachable_server_is_a_warning_not_a_failure() {
    // Spec §9: check and plan do not need the server, so doctor must not
    // report a red state for a machine that simply has not started it.
    let report = doctor_report_for("http://127.0.0.1:1").await;
    let line = report.voice_probe.expect("probe ran");
    assert!(line.contains("unreachable"), "{line}");
    assert!(report.ok, "unreachable is a warning, not an error");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p teleprompt-cli --test doctor doctor_reports_a_reachable`
Expected: FAIL — `voice_probe` does not exist and `doctor_report` is not async.

- [ ] **Step 3: Make `doctor_report` async and add the probe**

Add to `DoctorReport`:

```rust
    /// One line about the configured backend's server, or `None` when the
    /// resolved backend has nothing to probe (`null`).
    pub voice_probe: Option<String>,
```

Change the signature to `pub async fn doctor_report(registry: &SceneRegistry) -> DoctorReport` and add, before constructing the report:

```rust
    let voice_registry = crate::voice::registry_for(&backend_settings()).unwrap_or_else(|_| {
        // A bad `backends:` value is reported by `check`, with a span.
        // `doctor` is the command you run when nothing else works, so it
        // falls back to defaults rather than refusing to say anything.
        crate::voice::registry()
    });

    let voice_probe = match voice_registry
        .get("kokoro")
        .and_then(|b| b.as_any().downcast_ref::<KokoroVoice>().map(|k| k.base_url().to_string()))
    {
        None => None,
        Some(url) => {
            let k = voice_registry.get("kokoro").unwrap();
            let k = k.as_any().downcast_ref::<KokoroVoice>().unwrap();
            Some(match k.voices().await {
                Ok(vs) => format!("{url} — reachable, {} voices", vs.len()),
                Err(e) => format!("{url} — unreachable ({e})"),
            })
        }
    };
```

Add `backend_settings()` beside `cache_root()`, reading the discovered project's config and returning its `backends` map, or an empty map outside a project.

Render it after the `voice backends` line:

```rust
        if let Some(p) = &self.voice_probe {
            out.push_str(&format!("  voice kokoro     {p}\n"));
        }
```

- [ ] **Step 4: Build a runtime for the `Doctor` arm**

In `crates/teleprompt-cli/src/main.rs`, the `Doctor` arm now awaits. Wrap it the same way the `Dub` arm already is, reusing the existing helper if there is one:

```rust
        Command::Doctor => {
            let report = dub_runtime()?.block_on(doctor::doctor_report(&registry));
            /* ...existing rendering unchanged... */
        }
```

If the helper is named for `dub`, rename it to something honest like `runtime()` and update both call sites. `check`, `plan`, `diff`, and `new` must still build no runtime — that is the property Delivery A established and this task must not erode it.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p teleprompt-cli --test doctor && cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Confirm the inner loop is still runtime-free**

Run: `grep -n "block_on\|tokio::main" crates/teleprompt-cli/src/main.rs`
Expected: `block_on` in the `Dub` and `Doctor` arms only; no `#[tokio::main]`. Paste the output into the report.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-cli
git commit -m "feat(doctor): probe the configured voice server, warn when it is down"
```

---

## Task 7: Bounded concurrent synthesis with per-segment progress

Spec §7.2 and the §15 cold-cache risk. A local model server is the bottleneck, so fan-out is bounded and output order is the document's, not the completion order's.

**Files:**
- Modify: `crates/teleprompt-cli/src/cmd/dub.rs`
- Test: `crates/teleprompt-cli/tests/dub.rs`

**Interfaces:**
- Consumes: `KokoroVoice::concurrency()`.
- Produces: no new public API. `DubOutput` keeps its shape.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn segments_are_synthesized_concurrently_but_collected_in_document_order() {
    // Three segments against a stub that answers each with a distinguishable
    // length. Order in the manifest must follow the script regardless of
    // which reply lands first.
    let stub = kokoro_stub_varying_lengths().await;
    let p = project_with_config_and_script(
        &format!(
            "voice:\n  backend: kokoro\n  voice: af_heart\nbackends:\n  kokoro:\n    \
             base_url: \"{}\"\n    concurrency: 3\n",
            stub.base_url
        ),
        "First segment here.\n\nSecond segment here.\n\nThird segment here.\n",
    );

    let out = run_dub(&p.project, &p.script, "en", &p.out, false).await.unwrap();
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path(&p.out, "en")).unwrap())
            .unwrap();
    let ids: Vec<&str> = manifest["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();

    assert_eq!(ids.len(), 3);
    let sorted = { let mut c = ids.clone(); c.sort(); c };
    assert_eq!(ids, sorted, "segments must be in document order: {ids:?}");
    let _ = out;
}

#[tokio::test]
async fn a_failure_in_one_segment_fails_the_run() {
    // Spec §7.1. With fan-out it would be easy to collect errors and carry
    // on; a half-dubbed output directory is worse than none.
    let stub = kokoro_stub_failing_on_second().await;
    let p = project_with_config_and_script(
        &format!(
            "voice:\n  backend: kokoro\n  voice: af_heart\nbackends:\n  kokoro:\n    \
             base_url: \"{}\"\n",
            stub.base_url
        ),
        "One.\n\nTwo.\n\nThree.\n",
    );
    let err = run_dub(&p.project, &p.script, "en", &p.out, false).await.unwrap_err();
    assert!(matches!(err, DubError::Runtime(_)));
    assert!(!manifest_path(&p.out, "en").exists(), "no partial output");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p teleprompt-cli --test dub segments_are_synthesized_concurrently`
Expected: FAIL — the helpers do not exist; then, once written, the serial loop passes the order assertion but not the concurrency the test is named for. Make the stub record concurrent in-flight requests and assert the peak was above 1, so the test fails against a serial loop rather than passing vacuously.

- [ ] **Step 3: Replace the serial render loop with bounded fan-out**

In `run_dub_with`, the current loop is `for detail in &compiled.narration { ... }`. Replace with:

```rust
    // Bounded rather than unbounded: a local model server is the
    // bottleneck, and fanning out wider than it can serve makes the whole
    // run slower while making its failure modes worse.
    let limit = backend
        .as_any()
        .downcast_ref::<KokoroVoice>()
        .map(|k| k.concurrency())
        .unwrap_or(1);
    let permits = Arc::new(tokio::sync::Semaphore::new(limit));

    let mut tasks = Vec::with_capacity(compiled.narration.len());
    for (i, detail) in compiled.narration.iter().enumerate() {
        let permits = permits.clone();
        let backend = backend.clone();
        let cache = &cache;
        let detail = detail.clone();
        tasks.push(async move {
            let _permit = permits.acquire().await.expect("semaphore is never closed");
            let r = render_one(&backend, cache, &detail).await;
            // Progress is per segment because a cold 30-minute script is
            // minutes of silence otherwise (spec §15).
            if r.is_ok() {
                eprintln!("  [{}/{}] {}", i + 1, /* total */ 0, detail.segment_id);
            }
            r
        });
    }

    // `join_all` preserves input order in its results, so the document order
    // survives whatever order the server answers in.
    let rendered: Vec<Result<(String, Vec<u8>, u64), DubError>> =
        futures_util::future::join_all(tasks).await;

    let mut audio: Vec<(String, Vec<u8>, u64)> = Vec::with_capacity(rendered.len());
    for r in rendered {
        audio.push(r?);
    }
```

Extract the existing body of the loop into `async fn render_one(backend: &Arc<dyn VoiceBackend>, cache: &VoiceCache, detail: &NarrationDetail) -> Result<(String, Vec<u8>, u64), DubError>` without changing what it does — cache lookup, synthesize on miss, `cache.store`, return `(segment_id, wav_bytes, rendered_ms)`.

Fix the `total` placeholder above to `compiled.narration.len()`, captured before the loop. Do not leave a `0` in the output.

This needs `futures-util` (`default-features = false`, feature `std`) or, to avoid a dependency, `tokio::task::JoinSet` with the index carried through and results sorted by it afterwards. Prefer `JoinSet` if it avoids the new dependency — the workspace has held to a small dependency list and one crate for `join_all` is not worth breaking that. Say which you chose and why in the report.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p teleprompt-cli --test dub && cargo test --workspace`
Expected: PASS. The `null` path takes `limit = 1` and behaves exactly as before, so every existing `dub` test must be untouched. If any existing test needed changing, stop and explain why in the report rather than editing it.

- [ ] **Step 5: Commit**

```bash
git add crates/teleprompt-cli Cargo.lock
git commit -m "feat(dub): bounded concurrent synthesis, document-ordered output, per-segment progress"
```

---

## Task 8: Acceptance, the real-server test, and the docs

**Files:**
- Create: `crates/teleprompt-voice-kokoro/tests/real.rs`
- Modify: `README.md`, `docs/superpowers/specs/2026-08-15-teleprompt-voice-backends-design.md` (§9 sample output)

**Interfaces:** none new.

- [ ] **Step 1: Write the `#[ignore]`d real-server test**

`crates/teleprompt-voice-kokoro/tests/real.rs`:

```rust
//! The one test that needs a real Kokoro-FastAPI server. Not run in CI.
//!
//! Start one, then: `cargo test -p teleprompt-voice-kokoro --test real -- --ignored`

use teleprompt_voice::{SynthRequest, VoiceBackend};
use teleprompt_voice_kokoro::{KokoroConfig, KokoroVoice};

#[tokio::test]
#[ignore = "needs a Kokoro-FastAPI server on localhost:8880"]
async fn a_real_server_produces_plausible_audio() {
    let k = KokoroVoice::new(KokoroConfig::default()).unwrap();

    let voices = k.voices().await.expect("server reachable");
    assert!(!voices.is_empty(), "server reported no voices");

    let out = k
        .synthesize(&SynthRequest {
            text: "The quick brown fox jumps over the lazy dog.".to_string(),
            locale: "en".to_string(),
            voice: Some(voices[0].clone()),
            speed: 1.0,
        })
        .await
        .expect("synthesis succeeded");

    assert_eq!(out.pcm.sample_rate, 24_000);
    assert_eq!(out.pcm.channels, 1);
    // A nine-word sentence should land somewhere between half a second and
    // fifteen. Wider than any plausible voice, narrow enough to catch a
    // decode that produced garbage.
    let ms = out.pcm.duration_ms();
    assert!((500..15_000).contains(&ms), "implausible duration {ms}ms");
}
```

- [ ] **Step 2: Verify it is skipped by default**

Run: `cargo test -p teleprompt-voice-kokoro --test real`
Expected: `1 ignored`.

- [ ] **Step 3: Document the backend in the README**

Add a section covering, in prose rather than a bare table:

- the `backends.kokoro` settings and their defaults (`base_url`, `timeout_ms`, `concurrency`, `model`);
- that Kokoro must be running for `dub` only — `check`, `plan` and `diff` never contact it, which is why they work on a laptop with nothing installed;
- **that `base_url` and `model` are part of the cache key**, so pointing at a different server or model re-synthesizes the project once, and that `localhost` and `127.0.0.1` count as different servers for this purpose;
- that a broken server fails the run rather than substituting silence, and why;
- how to run the ignored integration test.

- [ ] **Step 4: Update spec §9's sample output**

The spec shows a `doctor` block. Make it match what the code now prints, including the `manifest` and `cache` lines that already exist. If it cannot be made to match exactly, change the code or the spec so it can — a sample output that is close but wrong is worse than none.

- [ ] **Step 5: Full verification**

Run each and paste the output into the report:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo +1.85 check --workspace
```

Expected: all clean. The last one is the CI MSRV job; `reqwest` and its TLS stack are the new risk, so this is not optional.

- [ ] **Step 6: Confirm the delivery's headline claim, by hand**

The spec claims adding a backend is one line in `crates/teleprompt-cli/src/voice.rs` plus one crate. Run:

```bash
git diff --stat af2482e..HEAD -- crates/teleprompt-core crates/teleprompt-compile crates/teleprompt-schedule crates/teleprompt-scene
```

Report exactly what changed outside the new crate and the CLI, and whether each change was genuinely forced by Kokoro or was a latent gap Kokoro merely exposed. **If the claim turns out to be false, say so plainly** — that is the single most valuable finding this delivery can produce, and reporting it accurately matters more than the claim holding.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-voice-kokoro/tests/real.rs README.md docs/superpowers/specs/2026-08-15-teleprompt-voice-backends-design.md
git commit -m "docs(kokoro): settings, the cache coupling, and the real-server test"
```

---

## Self-Review

**Spec coverage.** §7 config → T1, T2, T4. §7 request shape and `pcm` decode → T3. §7 capabilities including `word_timings: false` → T3. §7 voice listing → T3, T5 (amended: `dub`, not `check`). §7.1 no silent fallback → T3 (error mapping), T7 (one failure fails the run). §7.2 concurrency and document order → T7. §8 async boundary → T6 Step 6 guards it; no task adds async below the CLI. §9 `doctor` → T6. §10 determinism → T2's `version_string` is the whole mechanism. §11 testing → T3 stub, T8 ignored test; the round-trip test §11 also names already exists from Delivery A. §14 Delivery B scope → all eight tasks. §15 cold-cache progress → T7.

**Two spec amendments made before planning**, both committed with the plan:
1. §7 said `check` validates the voice against the server "when reachable" — which contradicts §8's inner-loop property and would make `check` machine-dependent. Moved to `dub`.
2. §7's config was a top-level `kokoro:` table, which would have put a per-backend named field in `teleprompt-core::Config` — a hardcoded list contradicting §4.1's pluggability claim. Now a generic `backends:` map core never interprets.

**Known gaps, deliberately not planned.** Timeouts are per-request via the client, but there is still no cancellation: Ctrl-C during `dub` is a process kill, and the WAV-then-sidecar cache ordering is what makes that safe rather than any cooperative shutdown. `VoiceError::Other(String)` still swallows every failure class, so `dub` cannot distinguish a retryable 503 from a fatal 400 and does not retry at all. Both were deferred from Delivery A on the grounds that they should be designed with a real network backend in front of them — **this delivery is that backend**, so they should be reconsidered immediately after it lands, on evidence rather than speculation. They are out of scope here only to keep the "does the contract hold?" question answerable in isolation.

**Type consistency.** `KokoroConfig`, `KokoroVoice`, `KOKORO_SAMPLE_RATE`, `registry_for`, `version_string`, `voices`, `base_url`, `concurrency`, `as_any`, `render_one` are each defined in exactly one task and referenced by their defining names thereafter. `as_any` is added to the trait in T5 and consumed in T5, T6 and T7.

**One risk worth naming.** T5's `as_any` widens a contract this project spent a whole delivery narrowing. The alternative — a `list_voices` method on `VoiceBackend` returning `Option<Vec<String>>` — is worse, because it puts one backend's capability in every backend's signature. But a reviewer should push on whether the validation is worth the escape hatch at all, and the honest fallback if it is not is to drop voice validation entirely and let synthesis fail on the first segment with the server's own error message.
