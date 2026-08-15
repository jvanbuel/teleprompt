# teleprompt pluggable voice backends — design

**Status:** accepted, unimplemented
**Depends on:** `docs/superpowers/specs/2026-08-15-teleprompt-design.md` (the
core design; unqualified `§` references point there)
**Milestone:** M1

## 1. Summary

teleprompt gets a real voice. Doing that properly means sharpening the
`VoiceBackend` contract first, because the current one is shaped around the
only implementation that exists.

Three deliverables:

- **A one-method backend contract**, async, returning audio. Everything else a
  backend currently has to know about — duration prediction, cache keys — moves
  to teleprompt.
- **Backends in their own crates**, so adding one touches nothing else. `null`
  moves out too, which is how the contract gets proven from outside.
- **A content-addressed synthesis cache**, plus `duration_source` on narration,
  so `plan` and `diff` stay sub-second and offline against a backend that takes
  seconds per segment.

Then `teleprompt-voice-kokoro`, which is small once the above exists.

### Goals

- A backend author implements one async function: text in, samples out.
- The inner loop (§10: `plan`, `diff`) never calls a backend and never needs an
  async runtime.
- A timeline says which durations are measured and which are predicted.
- Adding a backend touches no crate but its own — the property §7.6 already
  names as the goal for scene adapters.

### Non-goals

- Runtime-loaded plugins. See §12.
- Voice cloning or the `cloned` tier. That is M3 and ElevenLabs.
- Streaming synthesis. `dub` and `build` are batch.

## 2. Why the contract changes

The current trait:

```rust
fn synthesize(&self, req: &SynthRequest) -> Result<SynthResult, VoiceError>; // duration, no audio
fn render_pcm(&self, req: &SynthRequest) -> Result<Option<Pcm>, VoiceError>; // audio
fn cache_key(&self, req: &SynthRequest) -> String;
```

Three problems, each visible in the code as it stands:

**`synthesize` returns a duration without producing audio.** Only `null` can do
that, because it predicts from a word-count model. Every real backend must run
the model to learn the duration. The interface encodes a capability exactly one
implementation has, which is the definition of an interface shaped around its
sole implementor.

**The split between duration and audio invites a specific bug, and already
caused it.** Finding C1 of the narration-manifest review was `dub` calling the
two methods with different arguments — `voice: None, speed: 1.0` against the
segment's resolved config — so the manifest published `duration_ms: 3250` beside
a 6500 ms WAV. It was fixed by threading one request to both calls. A contract
where the two cannot disagree is better than one where a test checks that they
don't.

**`cache_key` makes every backend author reason about teleprompt's caching.**
That is teleprompt's concern leaking through the seam.

Also `render_pcm` returns `Option<Pcm>`, where `Ok(None)` means "this backend
cannot produce audio". Nothing returns it. A state no implementation reaches is
a state that was never thought through.

## 3. The contract

`teleprompt-voice` holds this and nothing else.

```rust
#[async_trait]
pub trait VoiceBackend: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> VoiceCapabilities;

    /// Turn text into audio. This is the only thing a backend does.
    ///
    /// Duration is not returned, because it is not separable: it is
    /// `pcm.duration_ms()`. A backend cannot claim a length its samples do
    /// not have.
    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError>;
}

pub struct Synthesized {
    pub pcm: Pcm,
    /// Present only when `capabilities().word_timings` is true.
    pub word_timings: Option<Vec<WordTiming>>,
}
```

`SynthRequest`, `Pcm`, `WordTiming`, and `VoiceError` keep their current
shapes. `VoiceCapabilities` gains one field, `version: String` — the backend's
own version, not teleprompt's — because §5 needs it in the cache key and the
backend is the only thing that knows it. `Pcm` already carries its own `sample_rate` and
`channels`, which turns out to matter: Kokoro emits 24 kHz mono, not the 48 kHz
`null` produces, and nothing in the pipeline has to care.

`SynthResult`, `render_pcm`, and `cache_key` are removed.

### 3.1 Duration estimation is not a backend concern

```rust
pub trait DurationEstimator: Send + Sync {
    /// Predict how long this text takes to speak, without synthesizing.
    /// Must be fast enough for the inner loop and deterministic.
    fn estimate_ms(&self, req: &SynthRequest) -> u64;
}
```

The word-count model currently inside `NullVoice` becomes the default
`DurationEstimator`. It is what `plan` and `diff` use whenever a segment is not
in the cache.

Separating these is what lets the estimator stay synchronous and the backend
be async. A backend author never implements `DurationEstimator`; a project that
wants a better predictor for a particular voice may.

The estimator is not configurable in v1: the CLI always uses `WpmEstimator`
from `teleprompt-voice-null`. Making it selectable would be a config key with
one possible value, and the trait exists so the choice can be added later
without reshaping anything.

## 4. Crates

| Crate | Holds | Depends on |
|---|---|---|
| `teleprompt-voice` | The contract: trait, `SynthRequest`, `Synthesized`, `Pcm`, `WordTiming`, `VoiceCapabilities`, `VoiceError`, `DurationEstimator`, the WAV encoder, `VoiceRegistry`. **No backends.** | `teleprompt-core`, `async-trait`, `serde` |
| `teleprompt-voice-null` | `NullVoice` (silence at the estimated length) and `WpmEstimator`. | `teleprompt-voice` |
| `teleprompt-voice-kokoro` | `KokoroVoice`, HTTP against a local Kokoro-FastAPI. | `teleprompt-voice`, `reqwest`, `tokio` |
| `teleprompt-cache` | Content-addressed synthesis cache. | `teleprompt-core`, `teleprompt-voice` |

Moving `null` out is deliberate and is the main structural check on the design:
if the reference implementation cannot be written from outside
`teleprompt-voice` using only its public API, the contract is not a contract.

### 4.1 Registry

Mirrors `SceneRegistry` (`crates/teleprompt-scene/src/registry.rs`) exactly,
including its `BTreeMap` so `available()` is ordered and `doctor` output is
deterministic:

```rust
pub struct VoiceRegistry { backends: BTreeMap<String, Arc<dyn VoiceBackend>> }

impl VoiceRegistry {
    pub fn register(&mut self, backend: Arc<dyn VoiceBackend>);
    pub fn get(&self, id: &str) -> Option<Arc<dyn VoiceBackend>>;
    pub fn available(&self) -> Vec<&str>;
}
```

`Arc`, not `Box`: `dub` synthesizes segments concurrently (§7.2), so the
backend is shared across tasks.

The CLI wires the concrete set, the way it already does for scenes. Adding a
backend is one line there and one new crate — nothing else in the workspace
changes.

### 4.2 Selection

`VoiceConfig.backend` already exists in `teleprompt-core::config` and **is read
by nothing**. It becomes live: the CLI resolves it against the registry, and an
unknown name is a validation error at `check` time naming what is available —
the same shape as the existing unknown-adapter diagnostic.

`backend` defaults to `null`, so every existing script and test keeps working
unchanged.

## 5. The cache

`.teleprompt/cache/voice/`, already gitignored by `teleprompt new`.

```
.teleprompt/cache/voice/<key>.wav     the encoded audio
.teleprompt/cache/voice/<key>.json    duration_ms, sample_rate, channels, word timings
```

`<key>` is BLAKE3 over a canonical string that must include **everything that
changes the audio**:

```
<backend id> / <backend version> / <locale> / <voice or "-"> / <speed> / <hash of text>
```

The backend version is the backend's own, reported by a new
`VoiceCapabilities::version: String` — not teleprompt's. This is a deliberate
correction of a defect found in the manifest work: `NullVoice::cache_key`
embedded `env!("CARGO_PKG_VERSION")`, so every teleprompt release invalidated
every cached segment and would have failed every consumer's next `--check` with
"audio changed" on every segment. A cache key must turn over when the thing
that produces the audio changes, and not before.

The cache is a pure function of its key. It is never invalidated by time, and
`teleprompt cache clean` (§10, already in the CLI synopsis) is how it is
cleared.

## 6. Estimated versus measured

`NarrationEntry` gains `duration_source: "estimated" | "measured"`, mirroring
the field `ActionEntry` already carries.

- `check`, `plan`, `diff` — consult the cache. Hit: the real duration,
  `measured`. Miss: the `DurationEstimator`, `estimated`. **Never** call a
  backend. Always offline, always sub-second.
- `dub`, `build` — synthesize on a miss, populate the cache, emit `measured`
  throughout.

So the loop is: edit prose, `plan` instantly with predictions, `dub` when you
want to hear it, and every subsequent `plan` is both instant and truthful.

### 6.1 The estimated-to-measured transition is drift, and must read as such

The first `dub` after writing a segment changes its duration from a prediction
to a measurement. That moves the timeline, and `diff --exit-code` will fail —
correctly, because the video really did change.

It must not be mistaken for a content edit. `diff` gains a reason for it:

```
changed:
  deploy           4.9s → 5.3s  (now measured)
```

alongside the existing `text edited` and `audio changed`. Same precedence
principle as the manifest's reason classification: name the cause, because the
causes call for different responses. `now measured` calls for committing the
new timeline; `text edited` calls for reading the prose again.

`plan --format json` warns when any segment it emits is `estimated`, since a
timeline committed from a cold cache will drift on the next `dub`.

## 7. The Kokoro backend

Kokoro-FastAPI, an OpenAI-compatible server the user runs locally (Docker or
pip). teleprompt speaks HTTP to it and owns no Python.

```toml
voice:
  backend: kokoro
  voice: af_heart
  speed: 1.0
kokoro:
  base_url: "http://localhost:8880"
  timeout_ms: 30000
```

```
POST {base_url}/v1/audio/speech
  { "model": "kokoro", "input": <text>, "voice": <voice>,
    "response_format": "pcm", "speed": <speed> }
```

**`response_format: "pcm"`** — raw little-endian 16-bit samples, so the response
body is `Pcm.samples` after a byte-pair decode. No mp3 or wav decoder enters the
workspace. Kokoro emits **24 kHz mono**; `Pcm` carries that, the WAV encoder
already writes any rate, and the manifest's `audio.sample_rate` already reports
whatever the backend produced.

**`speed` is sent to the server**, so the voice is *generated* at that rate
rather than resampled. This is exactly the boundary core spec §6.2 insists on:
`voice.speed` is a synthesis parameter, and the scheduler still must never
write it to make a beat fit.

`GET {base_url}/v1/audio/voices` backs `doctor` and validates a configured
`voice` at `check` time when the server is reachable.

Capabilities: `cloning: false`, `cross_lingual: false`, `ssml: false`,
`speed_control: true`, `word_timings: **false**`. Kokoro-FastAPI does expose
word timings, but only on `POST /dev/captioned_speech` — a `/dev/` path is not a
stable interface to build a published manifest field on. Revisit when it
stabilises; the manifest already omits `words` when absent.

### 7.1 Failure is a runtime error, never a silent fallback

A server that is unreachable, slow, or returns non-200 fails the command with
exit 1 and a message naming the URL and the segment. It does **not** fall back
to `null`. The voice fallback ladder (§4.1) moves between *tiers* the author
asked for — `recorded` → `cloned` → `synthetic` — and a broken synthesizer is
not a tier, it is a broken tool. Silently substituting silence for a voice
would be the worst possible failure mode for this product.

### 7.2 Concurrency

`dub` synthesizes segments concurrently, bounded by a `kokoro.concurrency`
setting defaulting to 4. A local model server is the bottleneck and unbounded
fan-out makes it worse. Results are collected in document order so output stays
deterministic regardless of completion order.

## 8. The async boundary

`async-trait` on `VoiceBackend`, `tokio` (multi-thread, `rt` + `net` + `macros`)
in the CLI.

The boundary sits so that **the inner loop never crosses it**. `compile()` takes
a cache and an estimator, not a backend, and stays synchronous. `check`, `plan`,
and `diff` never start a runtime. Only `dub` and `build` enter async, at their
own entry points.

That is the test of whether §3's split is in the right place: if the async
boundary had to sit any lower, it would have infected `plan`.

## 9. `doctor`

```
teleprompt doctor
  scene adapters   mock
  voice backends   kokoro, null
  voice kokoro     http://localhost:8880 — reachable, 54 voices
  cache            .teleprompt/cache/voice — 12 entries, 3.4 MB
```

`doctor` probes the configured backend and reports unreachable as a warning
rather than an error, since `check` and `plan` do not need it.

## 10. Determinism and the drift gate

Kokoro is deterministic for a fixed input, model, and speed — but teleprompt
does not depend on that being true forever. The cache is what makes the pipeline
reproducible: once a segment is cached, its duration and bytes are fixed until
its key changes. A model upgrade changes `capabilities().version`, which changes
every key, which produces one honest wave of `audio changed` in the diff.

This is the same posture as §5.1's build cache: reproducibility comes from
content addressing, not from trusting a model to be stable.

## 11. Testing

The suite gains no network dependency and no model download.

- The contract, registry, cache, and estimator are pure and tested with `null`
  alone, exactly as today.
- `teleprompt-voice-null` is built against `teleprompt-voice`'s public API only.
  If it needs anything not exported, the contract is wrong and that is a
  finding, not a workaround.
- The Kokoro backend is tested against a **stub HTTP server in-process**
  (`tokio` + a one-route listener on an ephemeral port), asserting the request
  body it sends, the `pcm` decode, the 24 kHz `Pcm` it produces, and its
  behaviour on timeout, 500, and a truncated body. No real Kokoro in CI.
- One `#[ignore]`d integration test hits a real server, for a human to run with
  `--ignored` when they have one up.
- A round-trip test asserts cache hit and miss produce identical timelines apart
  from `duration_source`.

## 12. Deferred: runtime-loaded plugins

A backend could be any executable named `teleprompt-voice-<id>` on `PATH`,
speaking JSON over stdio. There is a real argument for it: the TTS ecosystem is
overwhelmingly Python, and every Rust backend for it will shell out or speak
HTTP anyway.

Deferred, and the narrow contract is what makes deferring safe — a process
protocol backend is one more implementation of a single-method trait, addable
without touching anything that exists. Building both extension mechanisms now
would mean two contracts to keep sharp instead of one.

## 13. What breaks

- `SynthResult`, `render_pcm`, `cache_key` are gone. The only implementor is
  `NullVoice`, and the only callers are `compile` and `dub`, both in this
  workspace.
- `compile()`'s signature changes: it takes a cache and an estimator instead of
  a `&dyn VoiceBackend`.
- `NarrationEntry` gains a field, so every committed timeline and the two
  snapshots change once.
- Nothing in the artifact format changes. No script, no front matter, no
  manifest field. `backend` already existed and was simply inert.

## 14. Delivery

Two pieces, because the second is small only if the first is done.

**A — the contract and the machinery.** The sharpened trait, the crate split,
`null` moved out, the registry, config-driven selection, the cache,
`duration_source`, and the `now measured` diff reason. Entirely hermetic,
testable with `null`, and worth shipping alone: it makes the timeline honest
about which numbers are predictions.

**B — Kokoro.** The backend crate, the stub-server tests, `doctor` probing,
concurrency. Small, isolated, and the first real proof that the contract admits
an implementation nothing in it was designed around.

## 15. Risks

**The cache becomes the reproducibility story, so cache bugs are correctness
bugs.** A wrong key that collides across voices would serve one voice's audio
for another. Mitigated by making the key include every input that changes the
audio, and by a test that varies each field independently and asserts the key
moves.

**A cold cache makes the first `dub` slow** — every segment through a local
model. Expected, but it should not be a surprise: `dub` reports progress per
segment rather than sitting silent.

**Kokoro-FastAPI is a third-party server with its own release cadence.** Its
`/v1/audio/speech` surface is OpenAI-compatible and therefore fairly stable; the
`/dev/` endpoints are not, which is why §7 declines to depend on one.
