# teleprompt ElevenLabs backend and the voice error taxonomy — design

**Status:** accepted, unimplemented
**Depends on:** `docs/superpowers/specs/2026-08-15-teleprompt-voice-backends-design.md`
(the pluggable-backend design; unqualified `§` references point there)
**Supersedes:** the deferred "cancellation and error taxonomy" item, and §6 of
`2026-08-16-teleprompt-local-backend-design.md` (the `VoiceCatalog` refactor,
which that document left standing when the rest of it was dropped)
**Milestone:** M3

## 1. Summary

A third voice backend, `elevenlabs`, speaking to a commercial hosted API — and
the contract changes that backend forces.

`kokoro` and `null` are both free, local, unauthenticated and unmetered. Every
property the contract quietly assumed, it assumed because nothing had
contradicted it. ElevenLabs contradicts four of them at once: it needs a
credential, it bills per character, it rate-limits, and it constrains speed to a
range narrower than the config layer validates. Two of those turn out to be
contract bugs rather than backend quirks.

### Goals

- `teleprompt dub` against ElevenLabs, with retries that survive a transient
  429 rather than losing a half-finished run.
- An error taxonomy that distinguishes "wait and try again" from "you are out of
  money" from "your config is wrong", because the three need different words and
  different exit behaviour.
- A speed range in the contract, so `check` rejects an unreachable speed
  offline instead of the backend silently clamping it.
- The manifest's `words` field populated for the first time.

### Non-goals

- Streaming synthesis. `dub` writes whole segments; there is nothing to stream
  to.
- Cancellation. Named in the deferred item alongside the taxonomy, and still
  deferred — it is a `dub` concern that no backend forces.
- Voice cloning workflows. The backend reports `cloning: true` and cloned
  voices appear in the catalog like any other; teleprompt does not create them.
- Auto-tuning concurrency from the server's own headers. See §8.

## 2. Two deliveries

**Delivery C — the contract.** `teleprompt-voice`, `null`, `kokoro`, `dub`. No
new backend.

**Delivery D — the crate.** `teleprompt-voice-elevenlabs`, building on C.

They are specified together and shipped separately, and the coupling is
deliberate. Designing an error taxonomy without a backend that actually
produces 401s, 402s and 429s is how the workspace got `as_any`: an abstraction
shaped by imagination, which then had to be removed. C's shape is argued
entirely from D's observed behaviour, and every variant in §3.1 exists because
§3.2 has a row for it.

## 3. Delivery C — the contract

### 3.1 `VoiceError` becomes a struct

```rust
#[derive(Debug, thiserror::Error)]
#[error("{backend}: {detail}")]
pub struct VoiceError {
    /// Which backend failed. A `String`, not `&'static str`, for the same
    /// reason the enum's `Unsupported` variant carried one: a crate serving
    /// several configured endpoints has no literal to name itself with.
    pub backend: String,
    pub kind: ErrorKind,
    /// Human-readable, already scoped to the backend by the `Display` impl —
    /// so it must not repeat the backend's name.
    pub detail: String,
    /// Only ever `Some` on `RateLimited`, and only when the server said so.
    pub retry_after: Option<std::time::Duration>,
}

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
    InvalidRequest,
    /// DNS, TLS, connection, timeout, 5xx.
    Transient,
    /// A 2xx whose body was not what the protocol promised.
    Protocol,
    /// The request asks for something this backend cannot do, decided
    /// locally without a request.
    Unsupported,
    /// A bug in the backend itself.
    Internal,
}

impl ErrorKind {
    pub fn retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Transient)
    }
}
```

Three decisions worth their justification:

**`backend` appears once.** The enum it replaces carried it per-variant, so
every new variant re-litigated whether to include it and `Other(String)` simply
lost it — which is why Kokoro's client hand-formats `"kokoro at {url}: {what}"`
into a bare string. Those become structured fields, so `doctor` can render a
failure differently from `dub` instead of both printing one pre-baked sentence.

**`retryable()` hangs off `ErrorKind`, not `VoiceError`.** Retryability is a
property of the classification. Putting it on the error would let two backends
disagree about whether a 429 is worth retrying, and the disagreement would be
invisible until one of them wasted an author's afternoon.

**`#[non_exhaustive]`.** A future backend will bring a kind nobody predicted.
That should not be a breaking change for a workspace where most callers only
ever ask `retryable()`.

**`Unsupported` and `InvalidRequest` are deliberately separate.** Both mean the
author must change something, but only `InvalidRequest` proves a server was
reached and a credential accepted — the first thing worth knowing when a `dub`
fails. `Unsupported` is reachable with no network at all.

### 3.2 The ElevenLabs mapping

Verified against ElevenLabs' published error reference, not inferred. Each row
is one test in D.

| HTTP | `detail.code` | `ErrorKind` | Retry |
|---|---|---|---|
| 401 | `invalid_api_key`, `missing_api_key`, `invalid_authorization_header` | `Auth` | no |
| 402 | `insufficient_credits` | `Quota` | no |
| 403 | — | `Auth` | no |
| 429 | `rate_limit_exceeded`, `concurrent_limit_exceeded` | `RateLimited` | yes |
| 400, 404, 409, 422 | — | `InvalidRequest` | no |
| 500, 503 | — | `Transient` | yes |
| timeout, connect, TLS, DNS | — | `Transient` | yes |
| 2xx, unparseable body | — | `Protocol` | no |

The single most valuable fact here is that **quota exhaustion is 402, not 429**.
Had it shared 429's status, the client would have to read `detail.code` to tell
"wait a moment" from "you are out of money", and getting that wrong means either
retrying a spent balance five times with backoff or failing instantly on a
transient limit. It does not, so status code alone drives classification and
`detail.code` only enriches `detail`.

Classification reads the status code. A body that does not parse as ElevenLabs'
`{"detail": {...}}` shape does not change the kind — the status is authoritative
and a garbled body is one more thing to put in `detail`, not a reason to
reclassify a 401 as `Protocol`.

### 3.3 Speed becomes a range

```rust
pub struct VoiceCapabilities {
    // ...
    /// The speeds this backend accepts. `None` means it does not vary speed.
    pub speed: Option<std::ops::RangeInclusive<f64>>,   // replaces speed_control: bool
}
```

| Backend | Range | Source |
|---|---|---|
| `null` | `None` | synthesizes nothing |
| `kokoro` | `Some(0.25..=4.0)` | Kokoro-FastAPI `RATE_MIN, RATE_MAX = 0.25, 4.0`, enforced by a Pydantic `Rate` type |
| `elevenlabs` | `Some(0.7..=1.2)` | documented, all voices and all models |

`teleprompt-core` validates `voice.speed` as finite and greater than zero and
nothing else — a floor that exists because `speed: 0` once produced a `u64::MAX`
duration and a panic. Against ElevenLabs, `voice.speed: 1.5` passes that
validation, gets clamped server-side to 1.2, and returns real audio whose
`measured` duration is entirely honest about a speed the author did not ask for.
The output is wrong and every signal says it is fine. That is the failure mode
this project has repeatedly singled out as its worst, and a boolean cannot
express the constraint that prevents it.

**Where it is enforced matters as much as the type.** `capabilities()` is
synchronous, offline, and infallible, so the check belongs in `check` — the
inner loop — not in `dub`. An author gets the error in under a second, with no
network call and no spend, in the same place they already see script errors.
The message names the range and the backend.

`speed_control: bool` had exactly one consumer worth preserving, and
`speed.is_some()` answers it.

### 3.4 Retry lives in `dub`

Backends classify. They never loop.

```rust
pub struct RetryPolicy {
    pub max_attempts: u32,      // 4
    pub base_delay: Duration,   // 500ms
    pub max_delay: Duration,    // 30s
}
```

Delay for attempt `n` is `min(max_delay, base_delay * 2^n)`, then jittered.
`retry_after` overrides the computed delay when the server supplied one.

`dub` owns this because `dub` owns the fan-out, the concurrency semaphore, and
the progress output. A retry loop inside a client is invisible to progress —
the author sees a stalled segment with no explanation — and it fights the
semaphore above it, holding a permit while sleeping. One helper in
`teleprompt-voice`, used by `dub`, not reimplemented per crate.

**Jitter is seeded from the segment's cache key, not from a RNG.** Jitter exists
to decorrelate concurrent retries; deriving it from the key that is already
distinct per segment achieves that while keeping the suite reproducible and
avoiding a `rand` dependency. A test that reruns a 429 storm gets the same
schedule twice, which is the difference between a test that pins behaviour and
one that merely usually passes.

`dub` reports a retry on its progress line rather than hiding it. A run that
succeeded only after backing off three times is information the author should
have before their next run.

### 3.5 Migrating `null` and `kokoro`

Most of C's diff. `null` gains `speed: None` and constructs the struct form.
Kokoro's `fail()` helper collapses into structured fields, its `base_url` moving
from a hand-formatted string prefix into `backend`/`detail`. Its `decode_pcm`
rejections become `Protocol`; its transport failures become `Transient`; its
non-2xx becomes `InvalidRequest` or `Transient` by status.

Kokoro gains no retry behaviour of its own — it inherits `dub`'s, which is a
behaviour change worth stating: a local Kokoro server that briefly refuses a
connection now survives where it previously failed the command.

The contract conformance suite (`teleprompt-voice/tests/contract.rs`) gains one
invariant: **if `capabilities().word_timings` is true, `synthesize` must return
`Some`.** Today that is a doc comment on `Synthesized`, and today nothing
returns `true`, so nothing has ever tested it. D makes it live.

## 4. Delivery D — the crate

### 4.1 Layout

```
crates/teleprompt-voice-elevenlabs/
  src/lib.rs        re-exports
  src/config.rs     ElevenLabsConfig, version_string
  src/client.rs     HTTP, error classification
  src/timings.rs    character alignment -> word timings
  src/backend.rs    VoiceBackend + VoiceCatalog impls
```

Depends on `teleprompt-voice` and **not** `teleprompt-core`, like its siblings.
`timings.rs` is its own module because it is pure, total, and the part most
worth testing in isolation — it takes three arrays and the request text and
returns words or an error, with no HTTP anywhere near it.

### 4.2 Configuration

```toml
[backends.elevenlabs]
model = "eleven_multilingual_v2"
output_format = "pcm_24000"
concurrency = 2
timeout_ms = 60000
stability = 0.5
similarity_boost = 0.75
style = 0.0
speaker_boost = true
```

`output_format` defaults to `pcm_24000`, which is available on **every**
subscription tier — only 44.1 kHz PCM and WAV require Pro. Raw little-endian
16-bit mono, so no mp3 or wav decoder enters the workspace and Kokoro's standing
promise on that point survives a second HTTP backend.

`concurrency` defaults to **2**, against Kokoro's 4. Free-tier concurrency
limits are low, and the retry path should be an exception rather than the normal
operating mode; an author on a higher tier raises it.

`base_url` defaults to `https://api.elevenlabs.io` and stays overridable — the
stub-server tests need it, exactly as Kokoro's do.

### 4.3 The API key

Read from the environment variable `ELEVENLABS_API_KEY`. Sent as the
`xi-api-key` header. Nothing else.

**There is no `api_key` config field, and its absence is enforced.** The config
struct is `#[serde(deny_unknown_fields)]`, so `api_key = "..."` in
`teleprompt.toml` is a hard error whose message names the environment variable.
Silently ignoring it would be worse than either alternative: an author who typed
a secret into a version-controlled file learns at once instead of at
`git push`. This project's entire premise is that the config file lives in git.

The key is read at construction and its presence recorded; its **absence is not
a construction failure**. `Backends` already defers backend construction
failures so that a project using `null` is not broken by another backend's bad
settings, and an unset variable on a machine that will never dub is the same
case. `synthesize` and the catalog fail with `Auth` when the key is missing,
naming the variable.

The key is never logged, never rendered by `doctor`, and never folded into the
cache key.

### 4.4 Cache identity

`version_string()` folds in `model`, `output_format`, `stability`,
`similarity_boost`, `style`, `speaker_boost`, and the host. Floats are formatted
`{:.3}` so the string is stable across runs.

The voice settings are the trap. Change `stability` and the audio changes while
the `SynthRequest` — text, locale, voice, speed — is byte-identical. Omitting
them serves the old audio under the new settings permanently, reported as
`measured`, with nothing above the cache able to notice. This is the precise
hazard `VoiceCapabilities::version`'s doc comment describes, and the four voice
settings are this backend's instance of it.

**The API key is not in the key**: the same request from a different account
produces the same audio, and putting a secret through a hash that ends up in a
path is a poor idea independent of that.

### 4.5 `VoiceCatalog`, and canonicalizing voices

Authors will write `voice: "Rachel"`, not `voice: "21m00Tcm4TlvDq8ikWAM"`.

That is a problem the cache makes sharp: `teleprompt_cache::key` hashes
`SynthRequest.voice` verbatim, so a display name is an unstable key. Rename a
cloned voice and the key survives while the audio changes; use two spellings of
one name and the same audio is synthesized and billed twice.

So **`dub`'s pre-flight canonicalizes the configured voice to its stable id
before any `SynthRequest` is built**, and the cache is always keyed on the id.
The manifest records the id too, which makes a published manifest reproducible
across a rename.

That requires the voice list, which is the capability
`2026-08-16-teleprompt-local-backend-design.md` §6 proposed and left standing:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    /// Stable identity. What goes in the cache key.
    pub id: String,
    /// What a human writes. May change; may collide.
    pub name: String,
}

#[async_trait]
pub trait VoiceCatalog: Send + Sync {
    /// The voices a script may name. Network, and therefore never called
    /// by `check`.
    async fn voices(&self) -> Result<Vec<Voice>, VoiceError>;

    /// Where those voices come from, for diagnostics: a URL for a
    /// server-backed backend, a directory for a local one. Never fails and
    /// never touches the network — a diagnostic about an unreachable server
    /// has to be able to name it.
    fn origin(&self) -> String;
}
```

`VoiceRegistry` gains `register_with_catalog` and
`catalog(id) -> Option<Arc<dyn VoiceCatalog>>`.

**Matching happens once, in `dub`, not per backend**, so no two backends can
resolve a name differently:

1. exact match on `id`
2. else case-insensitive match on `name`, which must be unique
3. else an error listing the candidates — and, on an ambiguous name, listing
   every colliding id, because "pick one" is not advice an author can act on

Kokoro maps each server string to `Voice { id: s.clone(), name: s }` and behaves
exactly as it does today. `null` registers no catalog, which is what makes
`doctor` print no probe line for a default project — behaviour that already cost
one fix round and must survive this refactor, pinned by a test on rendered
output rather than on JSON.

`Backends::kokoro(id)` is deleted. Its third caller, the fan-out limit, moves to
`Backends::fan_out: BTreeMap<String, usize>`, populated per backend from its own
settings slice. Concurrency is configuration, not capability, and that was the
reason the accessor could not generalize.

### 4.6 Synthesis and word timings

`POST /v1/text-to-speech/{voice_id}/with-timestamps` with
`output_format=pcm_24000`. The response is JSON carrying `audio_base64` plus two
alignment objects; the audio inside is still raw PCM, so the format promise
holds and the only new cost is base64 — about 1.33× the audio in memory, under
2 MB for a 30-second segment.

This takes one dependency: `base64` (MIT/Apache-2.0, no transitive
dependencies). Hand-rolling a decoder to avoid it would be a poor trade — it is
exactly the kind of code that is subtly wrong on padding and never noticed.

**Use `alignment`, not `normalized_alignment`.** The normalized variant
describes text the API rewrote (`"$5"` becoming `"five dollars"`); the author's
words are in the original, and the manifest is about the author's script.

Words are maximal runs of non-whitespace characters. A word's start is its first
character's start; its end is its last character's end. Punctuation stays
attached to the word it touches — the word string is simply the run of
characters, which keeps the output reconstructible from the input.

Four invariants, each a `Protocol` error. This is where silent wrongness would
otherwise live, because a plausible-looking wrong alignment is published as
fact:

1. `characters`, `character_start_times_seconds` and
   `character_end_times_seconds` have equal length.
2. `characters` concatenated equals the request text **exactly**. Catches
   normalization leaking into the wrong field, and catches an API change.
3. Start and end times are non-decreasing.
4. The final end time matches `pcm.duration_ms()` within tolerance. The
   strongest of the four: it is what proves the alignment describes *this*
   audio rather than some other request's.

Tolerance is derived from measured error against the real API and justified in
a comment, not guessed.

`capabilities().word_timings` is `true`, so §3.5's new conformance invariant
binds: `synthesize` must return `Some`. An empty-text request cannot arise —
`check` rejects empty narration upstream.

### 4.7 Cost, reported

`dub` already groups segments by cache key before fanning out, so before
synthesizing anything it knows exactly which requests will be sent. It prints
the segment count and the character total. A re-run correctly reports near zero,
which is the number that makes the cache's value visible.

Character counting is local and deterministic. No response header is parsed for
this, so the report does not depend on an undocumented header name surviving.

Pre-flight already makes one network call to fetch the catalog; it also fetches
`GET /v1/user/subscription` and **warns when the remaining allowance is below
what this run needs**. Still reporting, not gating: the run proceeds. Knowing
before a twenty-minute dub beats discovering it at 80%.

### 4.8 `doctor`

Reports the tier, characters used against the limit, and whether
`ELEVENLABS_API_KEY` is set — never its value, not even truncated. An unset key
is reported as a plain fact, not an error, because a project may legitimately
configure `elevenlabs` on a machine that only ever runs `check`.

## 5. Testing

**Stub server**, following Kokoro's `tests/stub/mod.rs` pattern.

1. **One test per row of §3.2.** Each asserts the `ErrorKind`, and that
   `retryable()` agrees with the table.
2. **Retry behaviour**: a stub that 429s twice then succeeds completes; one that
   402s does not retry; one that 429s forever fails after `max_attempts` with
   the last error. Jitter's determinism makes the delay schedule assertable.
3. **Word timings**: a golden alignment fixture through `timings.rs`, plus one
   negative test per invariant in §4.6 — mismatched array lengths, characters
   that do not reconstruct the text, times that go backwards, and a final time
   that disagrees with the sample count.
4. **Cache identity**: flipping `stability` changes `version_string()`;
   changing the API key does not.
5. **Voice resolution**: id match, case-insensitive name match, ambiguous name
   listing both ids, unknown voice.
6. **Speed range in `check`**: `voice.speed: 1.5` against `elevenlabs` fails
   offline with no network call; the same value against `kokoro` passes.
7. **`doctor` prints no probe line** for a `null`-backend project, asserted on
   rendered output.
8. **`real.rs`**, gated on `ELEVENLABS_API_KEY` being set, ignored by default —
   Kokoro's precedent. It is the only place tolerance in §4.6 can be measured.

## 6. Risks

**The alignment tolerance is a guess until measured.** Invariant 4 is the
strongest check in the design and the only one whose threshold cannot be derived
from the protocol. Set too tight it fails on valid audio; too loose it stops
catching anything. It must be measured against the real API in `real.rs` before
a number is committed, and the comment must say what was measured.

**A run can partially spend money and then fail.** Retry reduces this; it does
not remove it. A `dub` that dies on segment 90 of 100 has paid for 89, and the
cache keeps them — so a re-run costs only the remainder. This is the strongest
practical argument for the cache existing at all, and it should be said in the
docs where an author will find it.

**Voice names are not stable identities.** §4.5 canonicalizes at pre-flight,
which fixes the cache key. It does not fix an author who renames a voice between
runs and expects the old name to keep working; that fails at pre-flight with a
clear message, which is the correct outcome but will surprise someone.

**`#[non_exhaustive]` hides missing arms.** A new `ErrorKind` will not break
compilation in the workspace, which is the point — but it also means a `match`
that should have grown an arm silently falls through to its wildcard. Every
wildcard arm over `ErrorKind` must be written to be correct for an unknown kind,
not merely to compile.

**ElevenLabs may change `detail.code` strings.** Classification depends on
status codes, which are stable; `detail.code` only enriches the message. This is
deliberate, and it is why the mapping is status-driven.

## 7. What breaks

- `VoiceError` changes from enum to struct. Every construction site in the
  workspace changes; every `match` on it changes.
- `VoiceCapabilities::speed_control: bool` becomes
  `speed: Option<RangeInclusive<f64>>`.
- `Backends::kokoro(id)` is removed. Its three callers move to
  `registry.catalog(id)` and `Backends::fan_out`.
- `VoiceRegistry` gains catalog registration. Kokoro's `voices()` moves from an
  inherent method to a trait impl returning `Vec<Voice>`; `base_url()` is
  exposed through `origin()`.
- `dub` gains a pre-flight report and a retry loop, and canonicalizes the voice
  before building requests.
- The manifest's `words` field is populated for the first time. It remains
  absent for backends that report `word_timings: false`, so existing Kokoro and
  `null` projects see **no manifest change and no drift**.
- Nothing else in the artifact format changes.
- `teleprompt-compile`, `teleprompt-schedule` and `teleprompt-scene` stay
  untouched. An implementer who finds themselves editing any of the three should
  report it as a finding rather than make the change quietly.

## 8. Deferred, deliberately

**Auto-tuning concurrency.** Responses carry `current-concurrent-requests` and
`maximum-concurrent-requests`, so the backend could discover its own ceiling
instead of taking it from config. Backoff already handles the failure, and this
would put a feedback loop between the client and `dub`'s semaphore for a problem
that may not appear in practice. Revisit if 429s prove common.

**Cancellation.** Named alongside the taxonomy in the original deferred item.
Nothing in this design forces it: no backend needs it, and `dub`'s fan-out is
where it belongs. It should be specified against `dub`, not against a backend.

**Streaming.** `dub` writes whole segments to a cache. There is nothing to
stream to until something consumes partial audio.

## 9. Delivery order

**C** — `VoiceError` struct and `ErrorKind`; the speed range and its `check`
enforcement; the retry helper and `dub`'s use of it; `null` and `kokoro`
migrated; the conformance invariant on `word_timings`.

**D** — the crate, config and cache identity; `VoiceCatalog` and the deletion of
`Backends::kokoro`; the client and its error mapping; `timings.rs`; the
pre-flight cost report and `doctor`; then docs.
