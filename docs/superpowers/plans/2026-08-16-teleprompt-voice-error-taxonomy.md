# Voice Error Taxonomy and Speed Range Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `VoiceError`'s enum with a classified struct, turn `speed_control: bool` into a speed range enforced at `check` time, and give `dub` a retry loop — so that a backend can say "wait and try again" and be believed.

**Architecture:** `teleprompt-voice` grows a classified error type and a pure retry helper. Backends classify their own server's failures and never loop. `dub` owns the loop, because it owns the fan-out, the semaphore and the progress output. The speed range is enforced in `teleprompt-cli`'s `compile_script_with`, not in `teleprompt-core`, because core must not depend on `teleprompt-voice`.

**Tech Stack:** Rust 2021, `thiserror`, `async-trait`, `tokio` (new `time` feature on `teleprompt-voice`), `reqwest`.

**Spec:** `docs/superpowers/specs/2026-08-16-teleprompt-elevenlabs-design.md` (Delivery C — §3 and §7)

## Global Constraints

- MSRV **1.85**, edition 2021. Nothing newer than `Option::is_some_and`.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` must all pass at every commit.
- **`teleprompt-core` must not gain a dependency on `teleprompt-voice`.** `teleprompt-schedule` names `VoiceSource` and must not depend on the voice crate; that is why `VoiceSource` lives in core. Do not invert it.
- **Backend crates must not depend on `teleprompt-core`.** `null` and `kokoro` depend on `teleprompt-voice` alone, and that is the standing proof the contract admits an outside backend.
- No new third-party dependencies in this delivery. `base64` belongs to Delivery D.
- `teleprompt-compile`, `teleprompt-schedule` and `teleprompt-scene` stay untouched. An implementer who finds themselves editing one should report it as a finding, not make the change.
- Error `detail` strings must not repeat the backend's name — `Display` is `"{backend}: {detail}"` and already prefixes it.

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/teleprompt-voice/src/contract.rs` | `VoiceError` struct, `ErrorKind`, `VoiceCapabilities::speed` | 1, 2 |
| `crates/teleprompt-voice/src/retry.rs` | **new** — `RetryPolicy`, `with_retry`, deterministic jitter | 4 |
| `crates/teleprompt-voice/src/lib.rs` | re-exports | 1, 2, 4 |
| `crates/teleprompt-voice/Cargo.toml` | `tokio` with `time` | 4 |
| `crates/teleprompt-voice/tests/contract.rs` | conformance, incl. new `word_timings` invariant | 1, 2, 6 |
| `crates/teleprompt-voice/tests/stub/mod.rs` | shared test stub | 1, 2 |
| `crates/teleprompt-voice/tests/retry.rs` | **new** — retry behaviour under paused time | 4 |
| `crates/teleprompt-voice-null/src/null.rs` | `null` migration | 1, 2 |
| `crates/teleprompt-voice-null/tests/null.rs` | assertion on `speed_control` | 2 |
| `crates/teleprompt-voice-kokoro/src/client.rs` | classification of Kokoro's server | 1 |
| `crates/teleprompt-voice-kokoro/src/backend.rs` | capabilities | 1, 2 |
| `crates/teleprompt-voice-kokoro/tests/synth.rs` | error-shape assertions | 1 |
| `crates/teleprompt-cli/src/cmd/check.rs` | speed-range enforcement | 3 |
| `crates/teleprompt-cli/src/cmd/dub.rs` | retry wiring | 5 |
| `crates/teleprompt-cli/tests/backend_selection.rs` | test backends | 1, 2 |
| `docs/` | config and backend docs | 6 |

**Why tasks 1 and 2 are large and indivisible.** Changing `VoiceError` from an enum to a struct, and `speed_control: bool` to `speed: Option<RangeInclusive<f64>>`, each break every implementor in the workspace simultaneously. There is no intermediate state where the workspace compiles. Each is therefore one task and one commit covering every call site, and the TDD cycle inside them is "write the new contract test, watch the workspace fail to compile, migrate until it passes".

---

### Task 1: `VoiceError` becomes a classified struct

**Files:**
- Modify: `crates/teleprompt-voice/src/contract.rs` (replace the `VoiceError` enum, ~lines 88-99)
- Modify: `crates/teleprompt-voice/src/lib.rs` (export `ErrorKind`)
- Modify: `crates/teleprompt-voice/tests/contract.rs:39,56`
- Modify: `crates/teleprompt-voice/tests/stub/mod.rs:39`
- Modify: `crates/teleprompt-voice-null/src/null.rs:43`
- Modify: `crates/teleprompt-voice-kokoro/src/client.rs` (the `fail` helper and all its callers)
- Modify: `crates/teleprompt-voice-kokoro/tests/synth.rs:107`
- Test: `crates/teleprompt-voice/tests/contract.rs`, `crates/teleprompt-voice-kokoro/tests/synth.rs`

**Interfaces:**
- Produces: `VoiceError { backend, kind, detail, retry_after }`, `ErrorKind` (8 variants, `#[non_exhaustive]`), `ErrorKind::retryable()`, and the constructors `VoiceError::new`, `VoiceError::unsupported`, `VoiceError::with_retry_after`. Tasks 4 and 5 consume `ErrorKind::retryable()` and `VoiceError::retry_after`.

- [ ] **Step 1: Write the failing test**

Add to `crates/teleprompt-voice/tests/contract.rs`:

```rust
#[test]
fn error_kinds_classify_retryability() {
    use teleprompt_voice::ErrorKind;
    assert!(ErrorKind::RateLimited.retryable());
    assert!(ErrorKind::Transient.retryable());
    for k in [
        ErrorKind::Auth,
        ErrorKind::Quota,
        ErrorKind::InvalidRequest,
        ErrorKind::Protocol,
        ErrorKind::Unsupported,
        ErrorKind::Internal,
    ] {
        assert!(!k.retryable(), "{k:?} must not be retryable");
    }
}

#[test]
fn error_display_names_the_backend_once() {
    let e = VoiceError::new(
        "kokoro",
        teleprompt_voice::ErrorKind::Transient,
        "no response within 30000ms",
    );
    assert_eq!(e.to_string(), "kokoro: no response within 30000ms");
}

#[test]
fn retry_after_is_absent_unless_set() {
    let e = VoiceError::new("x", teleprompt_voice::ErrorKind::RateLimited, "slow down");
    assert!(e.retry_after.is_none());
    let e = e.with_retry_after(std::time::Duration::from_secs(2));
    assert_eq!(e.retry_after, Some(std::time::Duration::from_secs(2)));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p teleprompt-voice --test contract`
Expected: FAIL — `ErrorKind` not found, `VoiceError::new` not found.

- [ ] **Step 3: Replace the enum in `contract.rs`**

Delete the `VoiceError` enum and put this in its place. Keep the existing doc comment's point about `backend` being a `String`, which now applies to the struct field:

```rust
/// What went wrong, and whether trying again could help.
///
/// `backend` is a `String`, not a `&'static str`: `id()` returns `&str`, so
/// a backend whose id is not a literal — one crate serving several
/// configured endpoints, which is the shape a network backend wants — could
/// not name itself in its own error. One allocation on an error path is the
/// whole cost.
///
/// This was an enum until a backend arrived that could fail in ways worth
/// telling apart. The enum carried `backend` per-variant, so each new
/// variant re-litigated whether to include it and `Other(String)` simply
/// lost it — which is why Kokoro's client hand-formatted its base URL into
/// a bare string. Those are fields now, so `doctor` can render a failure
/// differently from `dub` instead of both printing one pre-baked sentence.
#[derive(Debug, thiserror::Error)]
#[error("{backend}: {detail}")]
pub struct VoiceError {
    pub backend: String,
    pub kind: ErrorKind,
    /// Already scoped to the backend by the `Display` impl above, so it
    /// must not repeat the backend's name.
    pub detail: String,
    /// Only ever `Some` on `RateLimited`, and only when the server said so.
    pub retry_after: Option<std::time::Duration>,
}

/// How a failure should be treated. Deliberately about *treatment*, not
/// about HTTP: a backend that speaks no HTTP still classifies into these.
///
/// `#[non_exhaustive]` because the next backend will bring a kind nobody
/// predicted, and that should not be a breaking change for a workspace
/// where most callers only ever ask `retryable()`. The cost is that a
/// `match` which should have grown an arm falls through to its wildcard
/// instead of failing to compile — so every wildcard arm over `ErrorKind`
/// must be written to be correct for an unknown kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Credentials missing, malformed, or rejected.
    Auth,
    /// Credentials fine, allowance spent. Distinct from `RateLimited`
    /// because waiting does not help.
    Quota,
    /// Too many requests, or too many at once.
    RateLimited,
    /// The server rejected what we sent.
    ///
    /// Distinct from `Unsupported`: both mean the author must change
    /// something, but only this one proves a server was reached and a
    /// credential accepted, which is the first thing worth knowing when a
    /// `dub` fails.
    InvalidRequest,
    /// DNS, TLS, connection refused, timeout, 5xx.
    Transient,
    /// A 2xx whose body was not what the protocol promised.
    Protocol,
    /// The request asks for something this backend cannot do — decided
    /// locally, without a request.
    Unsupported,
    /// A bug in the backend itself.
    Internal,
}

impl ErrorKind {
    /// Whether trying the same request again could plausibly succeed.
    ///
    /// This hangs off `ErrorKind` rather than `VoiceError` on purpose.
    /// Retryability is a property of the classification, and putting it on
    /// the error would let two backends disagree about whether a 429 is
    /// worth retrying — a disagreement that stays invisible until one of
    /// them wastes an author's afternoon.
    pub fn retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Transient)
    }
}

impl VoiceError {
    pub fn new(backend: impl Into<String>, kind: ErrorKind, detail: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            kind,
            detail: detail.into(),
            retry_after: None,
        }
    }

    /// The request asks for something this backend cannot do.
    pub fn unsupported(backend: impl Into<String>, what: impl std::fmt::Display) -> Self {
        Self::new(backend, ErrorKind::Unsupported, format!("does not support {what}"))
    }

    pub fn with_retry_after(mut self, after: std::time::Duration) -> Self {
        self.retry_after = Some(after);
        self
    }
}
```

- [ ] **Step 4: Export from `lib.rs`**

In `crates/teleprompt-voice/src/lib.rs`, add `ErrorKind` to the `pub use contract::{...}` list.

- [ ] **Step 5: Migrate `null`**

`crates/teleprompt-voice-null/src/null.rs:43` — this is a locally-decided rejection with no request made, so it is `Unsupported`:

```rust
if req.speed <= 0.0 {
    return Err(VoiceError::new(
        "null",
        teleprompt_voice::ErrorKind::InvalidRequest,
        "speed must be greater than zero",
    ));
}
```

Use `InvalidRequest`, not `Unsupported`: `null` *can* vary speed, and this particular value is invalid. Add `ErrorKind` to the crate's `use teleprompt_voice::{...}` list.

- [ ] **Step 6: Migrate Kokoro's client**

Replace the `fail` helper in `crates/teleprompt-voice-kokoro/src/client.rs` with one that classifies. The base URL moves out of the message and into a `detail` suffix only where it adds information — `Display` already prints `kokoro: ...`, and the URL is still worth carrying because a machine may have several:

```rust
fn fail(&self, kind: ErrorKind, what: &str) -> VoiceError {
    VoiceError::new("kokoro", kind, format!("{} — {what}", self.cfg.base_url))
}

/// Kokoro-FastAPI is a local model server: it has no credentials and no
/// billing, so `Auth` and `Quota` are unreachable here. Anything not a
/// rate limit or a server fault is the request's problem.
fn classify(status: reqwest::StatusCode) -> ErrorKind {
    match status.as_u16() {
        429 => ErrorKind::RateLimited,
        500..=599 => ErrorKind::Transient,
        _ => ErrorKind::InvalidRequest,
    }
}
```

Then at each existing call site:

| Site | Kind |
|---|---|
| `e.is_timeout()` on send | `Transient` |
| other `reqwest` send error | `Transient` |
| `!status.is_success()` | `classify(status)` |
| body read timeout / incomplete | `Transient` |
| `decode_pcm` rejections | `Protocol` |
| `voices()` non-2xx | `classify(status)` |
| `voices()` body not JSON | `Protocol` |
| `voices()` no `voices` array | `Protocol` |
| `voices()` non-string entry | `Protocol` |

Honour `Retry-After` when the server sends it on a 429:

```rust
let mut err = self.fail(classify(status), &format!("returned {}{tail}", status.as_u16()));
if let Some(after) = resp
    .headers()
    .get(reqwest::header::RETRY_AFTER)
    .and_then(|v| v.to_str().ok())
    .and_then(|v| v.parse::<u64>().ok())
{
    err = err.with_retry_after(std::time::Duration::from_secs(after));
}
return Err(err);
```

Read the headers **before** `resp.text().await` consumes the response.

- [ ] **Step 7: Migrate the remaining call sites**

- `crates/teleprompt-voice/tests/stub/mod.rs:39` → `VoiceError::new("stub", ErrorKind::Unsupported, "stub backend cannot synthesize")`
- `crates/teleprompt-voice/tests/contract.rs:39,56` → the existing `VoiceError::Unsupported { backend, what }` constructions become `VoiceError::unsupported(backend, what)`. Update any assertion on the rendered string: it is now `"{backend}: does not support {what}"`.
- `crates/teleprompt-voice-kokoro/tests/synth.rs:107` — `assert!(!matches!(err, VoiceError::Unsupported { .. }))` becomes `assert_ne!(err.kind, ErrorKind::Unsupported)`.

- [ ] **Step 8: Add Kokoro classification tests**

In `crates/teleprompt-voice-kokoro/tests/synth.rs`, using the existing stub server:

```rust
#[tokio::test]
async fn server_500_is_transient_and_retryable() {
    let s = stub::Server::responding(500, "boom").await;
    let v = KokoroVoice::new(cfg_for(&s)).unwrap();
    let err = v.synthesize(&req("hello")).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Transient);
    assert!(err.kind.retryable());
}

#[tokio::test]
async fn server_429_carries_retry_after() {
    let s = stub::Server::responding_with_header(429, "slow down", "retry-after", "3").await;
    let v = KokoroVoice::new(cfg_for(&s)).unwrap();
    let err = v.synthesize(&req("hello")).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::RateLimited);
    assert_eq!(err.retry_after, Some(std::time::Duration::from_secs(3)));
}

#[tokio::test]
async fn odd_byte_count_is_protocol_not_transient() {
    let s = stub::Server::responding_bytes(200, &[0x01, 0x02, 0x03]).await;
    let v = KokoroVoice::new(cfg_for(&s)).unwrap();
    let err = v.synthesize(&req("hello")).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Protocol);
    assert!(!err.kind.retryable());
}
```

Follow the existing helpers in `tests/stub/mod.rs`; add `responding_with_header` and `responding_bytes` there if the current stub lacks them, matching its established style.

- [ ] **Step 9: Run the whole suite**

Run: `cargo test --workspace`
Expected: PASS.

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: clean.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat(voice): VoiceError becomes a classified struct

An enum with Other(String) could not say whether trying again would
help. Backends now classify; nobody loops yet."
```

---

### Task 2: Speed becomes a range

**Files:**
- Modify: `crates/teleprompt-voice/src/contract.rs` (`VoiceCapabilities`)
- Modify: `crates/teleprompt-voice-null/src/null.rs:36`, `crates/teleprompt-voice-null/tests/null.rs:93`
- Modify: `crates/teleprompt-voice-kokoro/src/backend.rs:59`
- Modify: `crates/teleprompt-voice/tests/stub/mod.rs:33`, `crates/teleprompt-voice/tests/contract.rs:30`
- Modify: `crates/teleprompt-cli/tests/backend_selection.rs:44,85`
- Test: `crates/teleprompt-voice/tests/contract.rs`

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `VoiceCapabilities::speed: Option<RangeInclusive<f64>>`. Task 3 consumes it.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn backends_declare_the_speeds_they_accept() {
    let null = teleprompt_voice_null::NullVoice::default();
    let r = null.capabilities().speed.expect("null varies speed");
    assert!(r.contains(&1.0));
}
```

Put this in `crates/teleprompt-voice-null/tests/null.rs`, replacing the `assert!(c.speed_control);` at line 93.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p teleprompt-voice-null`
Expected: FAIL — no field `speed` on `VoiceCapabilities`.

- [ ] **Step 3: Change the field**

In `crates/teleprompt-voice/src/contract.rs`, replace `pub speed_control: bool` with:

```rust
    /// The speeds this backend accepts, or `None` if it does not vary
    /// speed at all.
    ///
    /// A range rather than a boolean because a boolean cannot express the
    /// constraint that matters. `teleprompt-core` validates `voice.speed`
    /// as finite and positive and nothing else. A backend that clamps
    /// server-side — ElevenLabs caps at 1.2 — then returns real audio
    /// whose `measured` duration is entirely honest about a speed the
    /// author never asked for. The output is wrong and every signal says
    /// it is fine.
    ///
    /// `capabilities()` is synchronous, offline and infallible, so this is
    /// checkable in the inner loop: see `compile_script_with`.
    pub speed: Option<std::ops::RangeInclusive<f64>>,
```

**`VoiceCapabilities` must drop its `Eq` derive** — `RangeInclusive<f64>` is not `Eq`. Change `#[derive(Debug, Clone, PartialEq, Eq)]` to `#[derive(Debug, Clone, PartialEq)]`. If any call site required `Eq` (a `HashSet`, a `BTreeMap` key), that is a finding to report rather than work around.

- [ ] **Step 4: Set each backend's range**

| File | Value | Justification to put in a comment |
|---|---|---|
| `teleprompt-voice-null/src/null.rs` | `Some(0.25..=4.0)` | matches the estimator's usable band; `null` has no server to disagree |
| `teleprompt-voice-kokoro/src/backend.rs` | `Some(0.25..=4.0)` | Kokoro-FastAPI's `RATE_MIN, RATE_MAX = 0.25, 4.0`, enforced by a Pydantic `Rate` type |
| `teleprompt-voice/tests/stub/mod.rs` | `None` | the stub synthesizes nothing |
| `teleprompt-voice/tests/contract.rs` | `None` | same |
| `teleprompt-cli/tests/backend_selection.rs` (both) | `Some(0.25..=4.0)` | preserves today's `speed_control: true` behaviour |

- [ ] **Step 5: Run the suite**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS, clean.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(voice): speed support is a range, not a boolean

speed_control: bool could not express the constraint that a backend
clamping server-side silently discards the author's stated speed."
```

---

### Task 3: `check` enforces the speed range

**Files:**
- Modify: `crates/teleprompt-cli/src/cmd/check.rs` (in `compile_script_with`, after the backend resolves and the program compiles)
- Test: `crates/teleprompt-cli/tests/voice_selection.rs`

**Interfaces:**
- Consumes: `VoiceCapabilities::speed` from Task 2.
- Produces: nothing later tasks depend on.

**Why here and not in `teleprompt-core`.** `Config::problems()` is the natural-looking home and is the wrong one: `teleprompt-core` does not depend on `teleprompt-voice` and must not start, because `teleprompt-schedule` names `VoiceSource` and would inherit the dependency. `compile_script_with` has both the resolved backend and the compiled narration, and it already runs offline in `check`, `plan` and `diff`.

**Why over narration details and not the merged config.** `voice.speed` can be set per segment by an attribute, so the merged project config is not the only value that reaches a backend. `compiled.narration[i].synth_request.speed` is the value actually sent.

- [ ] **Step 1: Write the failing test**

In `crates/teleprompt-cli/tests/voice_selection.rs`, following the existing fixture helpers in that file:

```rust
#[test]
fn check_rejects_a_speed_the_backend_cannot_reach() {
    let p = project_with_config(
        r#"
[voice]
backend = "kokoro"
speed = 9.0
"#,
    );
    write_script(&p, "s.md", "Hello there.\n");
    let out = run_check(&p, "s.md");
    assert!(out.failed(), "expected check to fail, got: {out:?}");
    assert!(
        out.stderr.contains("voice.speed")
            && out.stderr.contains("9")
            && out.stderr.contains("0.25")
            && out.stderr.contains("4"),
        "message must name the value and the range: {}",
        out.stderr
    );
}

#[test]
fn check_accepts_a_speed_inside_the_range() {
    let p = project_with_config(
        r#"
[voice]
backend = "kokoro"
speed = 1.5
"#,
    );
    write_script(&p, "s.md", "Hello there.\n");
    assert!(run_check(&p, "s.md").succeeded());
}
```

Both run with no server listening — that is the point of the test.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p teleprompt-cli --test voice_selection check_rejects_a_speed`
Expected: FAIL — check succeeds.

- [ ] **Step 3: Implement**

In `compile_script_with`, after `let backend = backends.resolve(...)?;` and after the `CompileOutput` exists, before returning:

```rust
    // `capabilities()` is offline and infallible, so an unreachable speed
    // is caught here — in the inner loop, in under a second, with no
    // network call and no spend — rather than at `dub` time or, worse,
    // never: a backend that clamps server-side returns real audio whose
    // `measured` duration is honest about a speed the author never asked
    // for.
    //
    // Over narration details rather than the merged config because a
    // segment attribute can set `voice.speed`, so the project's merged
    // value is not the only one that reaches a backend.
    if let Some(range) = backend.capabilities().speed {
        let mut offenders: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for d in &out.narration {
            let s = d.synth_request.speed;
            if !range.contains(&s) {
                offenders
                    .entry(format!("{s}"))
                    .or_default()
                    .push(d.segment_id.clone());
            }
        }
        if !offenders.is_empty() {
            let diags: Vec<Diagnostic> = offenders
                .into_iter()
                .map(|(speed, segments)| {
                    Diagnostic::error(format!(
                        "voice.speed `{speed}` is outside the range backend `{}` accepts \
                         ({}..={}) — affects {}",
                        backend.id(),
                        range.start(),
                        range.end(),
                        segments.join(", ")
                    ))
                    .with_help(
                        "choose a speed inside the range, or a backend whose range \
                         covers it — a backend that clamps silently would publish a \
                         duration for a speed you did not ask for",
                    )
                })
                .collect();
            return Err(render(&Diagnostics(diags), &display));
        }
    }
```

Group by offending value and name every affected segment, matching the pattern the segment-backend check in this same function already uses — a chapter of twelve paragraphs under one stray attribute produces one error, not twelve.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p teleprompt-cli --test voice_selection`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(cli): check rejects a speed the resolved backend cannot reach

Offline, in the inner loop, before any network call or spend."
```

---

### Task 4: The retry helper

**Files:**
- Create: `crates/teleprompt-voice/src/retry.rs`
- Modify: `crates/teleprompt-voice/src/lib.rs`
- Modify: `crates/teleprompt-voice/Cargo.toml`
- Test: `crates/teleprompt-voice/tests/retry.rs`

**Interfaces:**
- Consumes: `ErrorKind::retryable()` and `VoiceError::retry_after` from Task 1.
- Produces: `RetryPolicy { max_attempts, base_delay, max_delay }`, `RetryPolicy::default()`, and
  `async fn with_retry<T, F, Fut>(policy: &RetryPolicy, seed: u64, op: F) -> Result<T, VoiceError>`
  where `F: FnMut(u32) -> Fut, Fut: Future<Output = Result<T, VoiceError>>`.
  Task 5 consumes both.

- [ ] **Step 1: Add the dependency**

In `crates/teleprompt-voice/Cargo.toml`:

```toml
[dependencies]
tokio = { workspace = true, features = ["time"] }

[dev-dependencies]
tokio = { workspace = true, features = ["time", "macros", "rt", "test-util"] }
```

Cargo unions features with the workspace's `["rt-multi-thread", "macros"]`. This is the one new dependency edge in this delivery, and it is on a crate the workspace already builds. Justify it in a comment: the helper must sleep, and its tests must control time.

- [ ] **Step 2: Write the failing tests**

Create `crates/teleprompt-voice/tests/retry.rs`:

```rust
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use teleprompt_voice::{with_retry, ErrorKind, RetryPolicy, VoiceError};

fn policy() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 4,
        base_delay: Duration::from_millis(500),
        max_delay: Duration::from_secs(30),
    }
}

#[tokio::test(start_paused = true)]
async fn succeeds_without_sleeping_when_the_first_attempt_works() {
    let r = with_retry(&policy(), 7, |_| async { Ok::<_, VoiceError>(42) }).await;
    assert_eq!(r.unwrap(), 42);
}

#[tokio::test(start_paused = true)]
async fn retries_a_transient_failure_then_succeeds() {
    let calls = AtomicU32::new(0);
    let r = with_retry(&policy(), 7, |_| {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            if n < 2 {
                Err(VoiceError::new("x", ErrorKind::Transient, "flaky"))
            } else {
                Ok(99)
            }
        }
    })
    .await;
    assert_eq!(r.unwrap(), 99);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test(start_paused = true)]
async fn does_not_retry_a_fatal_failure() {
    let calls = AtomicU32::new(0);
    let r: Result<(), _> = with_retry(&policy(), 7, |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Err(VoiceError::new("x", ErrorKind::Quota, "out of credits")) }
    })
    .await;
    assert_eq!(r.unwrap_err().kind, ErrorKind::Quota);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "Quota must not be retried");
}

#[tokio::test(start_paused = true)]
async fn gives_up_after_max_attempts_and_returns_the_last_error() {
    let calls = AtomicU32::new(0);
    let r: Result<(), _> = with_retry(&policy(), 7, |n| {
        calls.fetch_add(1, Ordering::SeqCst);
        async move { Err(VoiceError::new("x", ErrorKind::Transient, format!("attempt {n}"))) }
    })
    .await;
    let e = r.unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert_eq!(e.detail, "attempt 3", "the last error is the one returned");
}

#[test]
fn jitter_is_deterministic_and_spread() {
    // Same seed and attempt always gives the same fraction — this is what
    // makes a retry test reproducible instead of usually-passing.
    assert_eq!(
        teleprompt_voice::retry::jitter_fraction(42, 1),
        teleprompt_voice::retry::jitter_fraction(42, 1)
    );
    // Different segments decorrelate, which is the entire purpose.
    assert_ne!(
        teleprompt_voice::retry::jitter_fraction(42, 1),
        teleprompt_voice::retry::jitter_fraction(43, 1)
    );
    for seed in 0..100u64 {
        let f = teleprompt_voice::retry::jitter_fraction(seed, 2);
        assert!((0.0..1.0).contains(&f), "fraction out of range: {f}");
    }
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test -p teleprompt-voice --test retry`
Expected: FAIL — `with_retry` not found.

- [ ] **Step 4: Implement `retry.rs`**

```rust
//! Retry policy for voice synthesis.
//!
//! This lives here, and `dub` calls it — backends classify failures and
//! never loop. A retry loop inside a client is invisible to `dub`'s
//! progress output (the author sees a stalled segment with no
//! explanation) and it fights the semaphore above it, holding a permit
//! while it sleeps.

use std::future::Future;
use std::time::Duration;

use crate::contract::VoiceError;

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// A deterministic fraction in `[0, 1)` from a seed and an attempt number.
///
/// Full jitter needs a spread, not randomness as such: its purpose is to
/// decorrelate concurrent retries so a fleet of segments does not retry in
/// lockstep. Deriving it from the segment's own identity achieves that
/// while keeping the suite reproducible — a test that reruns a 429 storm
/// gets the same schedule twice, which is the difference between pinning
/// behaviour and usually passing. It also avoids a `rand` dependency.
///
/// splitmix64's finalizer. Not cryptographic and does not need to be.
pub fn jitter_fraction(seed: u64, attempt: u32) -> f64 {
    let mut z = seed.wrapping_add((attempt as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // 53 bits is f64's mantissa: every value is exactly representable.
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// A stable seed for a string identity, so the same segment jitters the
/// same way on every run. FNV-1a: six lines, no dependency, and — unlike
/// `DefaultHasher` — guaranteed stable across Rust releases, which a test
/// asserting a delay schedule depends on.
pub fn seed_from(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Run `op` until it succeeds, fails un-retryably, or runs out of attempts.
///
/// `op` receives the zero-based attempt number so a caller can log it. The
/// error returned on exhaustion is the *last* one, not the first: it is the
/// most recent description of what the server is actually doing.
pub async fn with_retry<T, F, Fut>(
    policy: &RetryPolicy,
    seed: u64,
    mut op: F,
) -> Result<T, VoiceError>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T, VoiceError>>,
{
    let mut attempt = 0u32;
    loop {
        match op(attempt).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                let last = attempt + 1 >= policy.max_attempts;
                if last || !e.kind.retryable() {
                    return Err(e);
                }
                // The server's own instruction wins over our guess.
                let delay = match e.retry_after {
                    Some(d) => d.min(policy.max_delay),
                    None => {
                        let backoff = policy
                            .base_delay
                            .saturating_mul(1u32 << attempt.min(16))
                            .min(policy.max_delay);
                        backoff.mul_f64(jitter_fraction(seed, attempt))
                    }
                };
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}
```

- [ ] **Step 5: Export**

In `crates/teleprompt-voice/src/lib.rs`:

```rust
pub mod retry;
pub use retry::{with_retry, RetryPolicy};
```

`retry` stays a public module so the tests can reach `jitter_fraction` and `seed_from` by path.

- [ ] **Step 6: Run to verify they pass**

Run: `cargo test -p teleprompt-voice --test retry`
Expected: PASS. The `start_paused` runtime makes the sleeps instant.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(voice): retry policy with deterministic jitter

Jitter is seeded from the caller's identity rather than a RNG: it
decorrelates concurrent retries while keeping the suite reproducible."
```

---

### Task 5: `dub` retries

**Files:**
- Modify: `crates/teleprompt-cli/src/cmd/dub.rs` (`render_one`, ~lines 210-267)
- Test: `crates/teleprompt-cli/tests/dub.rs`

**Interfaces:**
- Consumes: `with_retry`, `RetryPolicy`, `seed_from` from Task 4.

**Wrap only the network call.** The cache lookup and the store are local and deterministic; retrying them would retry a disk error that will not fix itself, and re-running `store` after a success would be wrong. Only `backend.synthesize` goes inside the closure.

- [ ] **Step 1: Write the failing test**

In `crates/teleprompt-cli/tests/dub.rs`, register a backend that fails transiently twice then succeeds — following the pattern `tests/backend_selection.rs` already uses to register a test backend via `Backends::from_registry`:

```rust
/// Fails with `Transient` the first two times it is asked, then succeeds.
struct FlakyVoice {
    calls: std::sync::atomic::AtomicU32,
}

#[async_trait]
impl VoiceBackend for FlakyVoice {
    fn id(&self) -> &str { "flaky" }
    fn capabilities(&self) -> VoiceCapabilities { /* speed: Some(0.25..=4.0), version: "1" */ }
    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            return Err(VoiceError::new("flaky", ErrorKind::Transient, "not yet"));
        }
        Ok(/* one frame of silence */)
    }
}

#[tokio::test]
async fn dub_survives_a_transient_failure() {
    // ... build a one-segment project against `flaky`, run dub ...
    assert!(result.is_ok());
    assert_eq!(backend.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn dub_does_not_retry_a_fatal_failure() {
    // same shape, but the backend returns ErrorKind::Quota every time
    assert!(result.is_err());
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
}
```

Keep `base_delay` small in these tests, or use `#[tokio::test(start_paused = true)]`, so the suite does not actually sleep.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p teleprompt-cli --test dub dub_survives`
Expected: FAIL — one call, and `dub` errors.

- [ ] **Step 3: Implement**

In `render_one`, replace the `backend.synthesize(...)` call:

```rust
            // Retry lives here rather than in the backend: `dub` owns the
            // fan-out, the semaphore and the progress output, so it is the
            // only layer that can report a retry to the author and the only
            // one that knows a sleeping task is holding a permit.
            //
            // Seeded from the segment id so each segment's backoff is
            // spread differently — that is what stops a whole fan-out from
            // retrying in lockstep — while staying identical run to run.
            let seed = teleprompt_voice::retry::seed_from(&detail.segment_id);
            let synthesized = teleprompt_voice::with_retry(policy, seed, |attempt| async move {
                if attempt > 0 {
                    eprintln!(
                        "  segment `{}`: retry {attempt} after a transient failure",
                        detail.segment_id
                    );
                }
                backend.synthesize(&detail.synth_request).await
            })
            .await
            .map_err(|e| DubError::Runtime(format!("segment `{}`: {e}", detail.segment_id)))?;
```

`render_one` takes a `policy: &RetryPolicy` parameter; `run_dub_with` constructs one `RetryPolicy::default()` and passes it down. Do not read it from config in this delivery — a knob nobody has asked for is a knob to add when they do.

The closure borrows `detail` and `backend`; if the borrow checker objects to `async move` capturing them, capture references outside the closure (`let backend = &backend; let detail = &detail;`) rather than cloning per attempt.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p teleprompt-cli --test dub`
Expected: PASS.

- [ ] **Step 5: Run the whole suite**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: PASS, clean.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(cli): dub retries transient synthesis failures

Kokoro inherits this: a local server that briefly refuses a connection
now survives where it previously failed the command."
```

---

### Task 6: The `word_timings` conformance invariant, and docs

**Files:**
- Modify: `crates/teleprompt-voice/tests/contract.rs`
- Modify: the backend/config documentation under `docs/` that lists `speed_control` or describes voice errors — locate with `rg -l 'speed_control|VoiceError' docs/`

**Interfaces:**
- Consumes: everything above.

**Why this test exists now.** `Synthesized`'s doc says word timings are "present only when `capabilities().word_timings` is true", and nothing has ever enforced it because nothing in the workspace returns `true`. Delivery D's ElevenLabs backend will, and the invariant should be in place and passing before a backend depends on it.

- [ ] **Step 1: Write the test**

In `crates/teleprompt-voice/tests/contract.rs`:

```rust
/// A backend that claims word timings and does not deliver them.
struct LyingVoice;

#[async_trait]
impl VoiceBackend for LyingVoice {
    fn id(&self) -> &str { "lying" }
    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities { word_timings: true, /* ...rest as the stub */ }
    }
    async fn synthesize(&self, _req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        Ok(Synthesized { pcm: silence(), word_timings: None })
    }
}

/// The invariant every backend must satisfy, exposed so a backend crate's
/// own tests can call it against their implementation.
pub async fn assert_word_timings_invariant(b: &dyn VoiceBackend, req: &SynthRequest) {
    let out = b.synthesize(req).await.expect("synthesis succeeded");
    if b.capabilities().word_timings {
        assert!(
            out.word_timings.is_some(),
            "backend `{}` claims word_timings but returned None",
            b.id()
        );
    }
}

#[tokio::test]
#[should_panic(expected = "claims word_timings but returned None")]
async fn a_backend_that_claims_timings_must_produce_them() {
    assert_word_timings_invariant(&LyingVoice, &req("hello")).await;
}

#[tokio::test]
async fn null_and_stub_satisfy_the_invariant() {
    assert_word_timings_invariant(&NullVoice::default(), &req("hello")).await;
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teleprompt-voice --test contract`
Expected: PASS — the `should_panic` test proves the assertion actually fires.

- [ ] **Step 3: Update the docs**

Find every place the docs describe backend capabilities or voice errors:

```bash
rg -l 'speed_control|VoiceError|voice\.speed' docs/
```

For each: replace `speed_control` with the speed range, note that `check` now rejects an out-of-range speed offline, and document that `dub` retries transient failures with backoff. Keep the existing document's voice and format — do not restructure a doc to make room.

- [ ] **Step 4: Full verification**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test(voice): pin the word_timings invariant before a backend needs it

Plus docs for the speed range and dub's retry behaviour."
```

---

## Self-Review

**Spec coverage.** §3.1 → Task 1. §3.2's mapping table is ElevenLabs-specific and belongs to Delivery D; Kokoro's own (narrower) classification is Task 1 Step 6. §3.3 → Tasks 2 and 3. §3.4 → Tasks 4 and 5. §3.5's migration → Tasks 1 and 2; its conformance invariant → Task 6.

**Not in this plan, deliberately:** everything in spec §4 (the ElevenLabs crate), `VoiceCatalog` and the deletion of `Backends::kokoro` (spec §4.5 — it is D's, because D is what needs `Voice { id, name }` and building it now would mean guessing the shape), and cancellation (spec §8).

**Task 2's `Eq` removal was checked, not assumed.** `VoiceCapabilities` appears in nine files; none compares two of them, uses one as a map or set key, or otherwise needs `Eq`. Dropping the derive is safe. If the compiler disagrees, that is a call site added after this plan was written — report it rather than reaching for a workaround.

**Known risk in Task 5.** The borrow shape inside `with_retry`'s closure is the one place a compile error is likely. The closure is `FnMut` and each attempt creates a new future; `detail` and `backend` must be borrowed, not moved. If it resists, the fallback is to change `with_retry`'s bound from `FnMut(u32) -> Fut` to accept a `&` capture explicitly — a signature change, which is a finding to report since Task 4's tests pin the current one.
