# teleprompt local Kokoro backend — design

**Status:** evaluated, not adopted (2026-08-16) — see [Disposition](#disposition)
**Depends on:** `docs/superpowers/specs/2026-08-15-teleprompt-voice-backends-design.md`
(the pluggable-backend design; unqualified `§` references point there)
**Milestone:** none

## Disposition

**This design was not built.** It is kept because the measurements in §2 and the
G2P finding below cost real time to obtain and would otherwise be rediscovered
at the same price. Everything after this section is the design as it stood when
the decision was taken — read it as the record of an evaluation, not as work
queued up.

### What stopped it

**Grapheme-to-phoneme is unsolved for this vocabulary.** §3 below claims
`misaki-rs` makes G2P a settled problem with no GPL exposure. A direct probe of
`misaki-rs` 0.3.0 with its optional `espeak-rs` dependency off shows otherwise:
out-of-vocabulary words are **spelled letter by letter**, silently, with no
marker and no error.

```
IN   teleprompt dubs narration.
OUT  tˈiː ˈiː ˈɛl ˈiː pˈiː ˈɑːɹ ˈoʊ ˈɛm pˈiː tˈiː  dˈʌbz nɚɹˈeɪʃən
IN   Kubernetes orchestrates zalgorithmic frobnication.
OUT  kˈA jˈuː bˈiː ˈiː ˈɑːɹ ˈɛn ˈiː tˈiː ˈiː ˈɛs  …
```

The words that fall outside a 100k-word lexicon — product names, tool names,
`Kubernetes`, `Postgres`, and `teleprompt` itself — are exactly the vocabulary a
narrated technical demo is made of. The failure is silent, so it reaches a
published video rather than a command exit. Every remedy costs something the
design was built to avoid: enabling `espeak-rs` links GPL code into an
MIT-licensed binary; shelling out to the `espeak-ng` executable restores a
process boundary, and a worse one than HTTP; a hand-authored lexicon is
unbounded ongoing work.

**The native dependency came back.** The original case for going in-process was
removing an external runtime. `ort` links ONNX Runtime and fetches a prebuilt
library at build time (§4.1). That is far lighter than Burn's 583 MB, but it is
the same category of cost as the thing being removed, now paid by everyone who
builds the workspace rather than only by users who want local synthesis.

Taken together: each fix restored a property the previous step had cost. The
HTTP `kokoro` backend already delivers local, offline, no-cost synthesis with
correct pronunciation; what this design would have added is the removal of one
process, and that turned out to be the most expensive property on the list.

### What the exploration established

The pluggable-backend contract holds. A second real backend landed in its own
crate with `teleprompt-compile`, `teleprompt-schedule` and `teleprompt-scene`
untouched, and the work removed `as_any` from `VoiceBackend` rather than adding
to it. That was the question worth answering.

### What would change the decision

- A phonemizer with permissive licensing that handles OOV words by rule rather
  than by spelling — or evidence that `misaki-rs` has grown one.
- [Piper](https://github.com/rhasspy/piper) instead of Kokoro: smaller models,
  ships its own phonemizer built for offline use. Same `ort` trade, better G2P
  story. This is the shape to revisit, not this document's.
- The `VoiceCatalog` refactor in §6 stands on its own merits and does not depend
  on any of the above. `Backends::kokoro(id)` remains an accessor that does not
  generalize; the next backend to need voice listing should do §6 regardless.

## 1. Summary

A third voice backend, `kokoro-local`, running Kokoro-82M **in process** via
ONNX Runtime — no HTTP server, no Python, no Docker. It sits alongside `null`
and the HTTP `kokoro` backend rather than replacing either.

The product property is a single binary: `cargo install teleprompt`, point it at
a model file, and dub. The HTTP backend keeps the eight languages this one
cannot pronounce.

Measured on a 4-core Xeon, 12 phoneme ids producing 31,800 samples (1.33 s at
24 kHz): **0.9 s model load, 0.32 s inference** — roughly 4× faster than real
time.

### Goals

- `teleprompt dub` with no external runtime.
- The inner loop (`check`, `plan`, `diff`) untouched: synchronous, offline,
  sub-second.
- `cargo test --workspace` stays fast and hermetic for anyone who never enables
  this backend.
- Retire `Backends::kokoro(id)` rather than extend it.

### Non-goals

- Replacing the HTTP backend.
- Non-English languages. See §7.3.
- GPU execution providers. ONNX Runtime supports them; this ships CPU only, and
  nothing here forecloses adding one.

## 2. Why not Burn

This design was originally written around [Burn](https://burn.dev) and
`burn-onnx`, on the strength of a spike that compiled and ran. It was rewritten
after measurement. The evidence is recorded here because it is the reason for
every major choice below, and because it would otherwise be rediscovered at
cost.

| Backend | Fusion | Load | Inference | Outcome |
|---|---|---|---|---|
| Burn `ndarray` + OpenBLAS + SIMD | none | 0.6 s | **9.0 s** | works, ~6.8× slower than real time |
| Burn `burn-cpu` (MLIR/CubeCL) | yes | 0.9 s | — | **SIGFPE** after MLIR lowering failure |
| **ONNX Runtime 1.28** | yes | 0.9 s | **0.32 s** | works, ~4.2× faster than real time |

Three findings, in the order they mattered:

**`burn-onnx` emits a literal transliteration.** All 2,463 nodes of Kokoro's
graph become 2,463 operations, each allocating a fresh tensor. ONNX Runtime
fuses and plans allocations first. Adding OpenBLAS and SIMD to the `ndarray`
backend moved inference only from ~12.7 s to 9.0 s — about 1.4× — which
establishes that **kernel quality was not the gap**. The remaining ~28× is graph
optimization and memory planning.

**Burn's fused CPU backend does not run this model.** `burn-cpu` 0.21 is
`Fusion<CubeBackend<CpuRuntime>>` with fusion on by default, so the fair
comparison was available. It fails during kernel compilation with
`error: 'arith.muli' op requires the same type for all operands and results`
inside `cubecl-cpu` 0.10's MLIR lowering, then dies with SIGFPE. This is a
reproducible upstream bug, not a configuration problem.

**`burn-cpu`'s dependency is heavier than the one it was meant to avoid.** It
pulls `tracel-llvm-bundler`, which downloads a **583 MB LLVM 20.1.4 toolchain**
at build time. That download ignores `HTTPS_PROXY` and the crate fails to build
on docs.rs. Since the entire argument for Burn was avoiding an external runtime,
a half-gigabyte build-time toolchain that breaks behind a proxy is a worse trade
than one bundled shared library.

Note for anyone revisiting: `libmlir-14` through `libmlir-20` *are* packaged on
Debian/Ubuntu, so system MLIR is not the obstacle — `tracel-llvm-bundler`
exposes no way to use it (its only feature is `xtask`), and the crash makes the
question moot regardless.

**What would change this.** A `burn-onnx` release that fuses, or a `cubecl-cpu`
that compiles this graph. Both are plausible; neither is today. The design below
keeps the runtime behind a narrow seam (§4) so swapping it later touches one
file.

## 3. Pipeline

```
text ──misaki-rs──> phonemes ──token map──> ids ─┐
                                                 ├─> ort session ──> PCM 24 kHz mono
                          voices/<id>.bin ───────┘
                          style[n_tokens]
```

Only the middle stage is new. The cache, WAV encoder, `duration_source`,
manifest and scheduler are reached through the existing `VoiceBackend` contract
and do not change.

G2P is [`misaki-rs`](https://crates.io/crates/misaki-rs) (MIT): a self-contained
Rust port of Kokoro's own Misaki engine, lexicons and POS-tagger weights
embedded at compile time. No `espeak-ng`, so no GPL, and the G2P stage needs no
files on disk.

> **This paragraph is wrong, and it is the reason the design was dropped.** The
> licensing claim holds only because the `espeak-rs` dependency is off, and with
> it off `misaki-rs` silently spells out-of-vocabulary words letter by letter.
> See [Disposition](#disposition). Left in place because the mistake — treating
> "a pure-Rust G2P crate exists" as "G2P is solved" — is the one worth not
> repeating.

## 4. Crate, runtime, and gating

New crate `crates/teleprompt-voice-kokoro-local/`. Like its siblings it depends
on `teleprompt-voice` and **not** on `teleprompt-core`.

`ort` is confined to one module, `src/session.rs`, behind a small internal
interface: token ids plus a style vector plus a speed in, `Vec<i16>` at 24 kHz
out. Nothing else in the crate names `ort`. §2 makes it likely someone will want
to swap the runtime within a year; this is what keeps that to one file.

### 4.1 The feature gate

`ort` links ONNX Runtime, and its default `download-binaries` feature fetches a
prebuilt library at build time. That is far lighter than Burn's 583 MB but it is
still network in a build, so the crate stays behind an off-by-default `runtime`
feature.

The gate must be **inside the crate**, not on the CLI: a feature flag on a
dependent does not stop `cargo test --workspace` from building a member. With
the feature off the crate compiles in seconds and `synthesize` returns a
`VoiceError` naming the feature.

Unlike the Burn design there is **no build-time codegen and no `build.rs`** —
`ort` loads the `.onnx` at runtime. That removes the checksum-pinned build step,
the 8.5-minute codegen, and the 8,827 lines of generated code with no review
surface.

### 4.2 Assets, all runtime

| Asset | Size | Source |
|---|---|---|
| `model.onnx` | 311 MB | `backends.kokoro-local.model_path` |
| `voices/*.bin` | 54 × 510 KB | `backends.kokoro-local.voices_dir` |

Both are runtime paths the author supplies. Nothing is downloaded by teleprompt
and nothing is vendored. `doctor` reports whether they are present and readable.

## 5. Configuration

```yaml
voice:
  backend: kokoro-local
  voice: af_heart
  speed: 1.0
backends:
  kokoro-local:
    model_path: "~/.local/share/teleprompt/kokoro/model.onnx"
    voices_dir: "~/.local/share/teleprompt/kokoro/voices"
    concurrency: 1
    threads: 0        # 0 = let ONNX Runtime choose
```

Settings arrive through the generic `backends:` map §4.1 established; core does
not learn this backend's name.

**`concurrency` defaults to 1.** ONNX Runtime parallelises within a single
inference, so `dub` fanning out across segments on top of that oversubscribes
the same cores. The generic `fan_out` plumbing handles it with no new machinery.
`threads` exposes ORT's intra-op pool for anyone who wants to trade the other
way.

### 5.1 Cache identity

`capabilities().version` **must fold in the SHA-256 of the model file**, hashed
once at construction. It is the only handle a backend has on its own cache key,
and two model files produce different audio for an identical request with
nothing above the cache able to tell.

Deliberately **not** in the key: `voices_dir` (the voice is already in the
`SynthRequest`, and moving a directory must not re-synthesize a project),
`model_path` (the same file at a new path is the same audio — the digest is what
matters), `concurrency`, and `threads`.

## 6. The `VoiceCatalog` capability

`dub` validates the configured voice before synthesizing anything and `doctor`
reports what a backend offers. Both reach `KokoroVoice` through
`Backends::kokoro(id)`, an accessor two reviewers flagged as not generalizing
past one server-backed backend, with the decision deferred to whoever added the
next one. This is that backend.

The accessor serves **three** needs across three call sites; all three must be
rehoused or it survives in another form:

| Call site | Needs | New home |
|---|---|---|
| `dub` pre-flight | `voices()`, `base_url()` for the error | `VoiceCatalog` |
| `doctor` probe | `voices()`, `base_url()` for the line | `VoiceCatalog` |
| `dub` fan-out | `concurrency()` | `Backends::fan_out` |

`VoiceBackend` stays at one method:

```rust
#[async_trait]
pub trait VoiceCatalog: Send + Sync {
    /// The voice ids a script may name for this backend.
    async fn voices(&self) -> Result<Vec<String>, VoiceError>;

    /// Where those voices come from, for diagnostics: a URL for a
    /// server-backed backend, a directory for a local one. Never fails and
    /// never touches the network — a diagnostic about an unreachable server
    /// has to be able to name it.
    fn origin(&self) -> String;
}
```

`VoiceRegistry` gains `register_with_catalog` and
`catalog(id) -> Option<Arc<dyn VoiceCatalog>>`. HTTP Kokoro implements it via
`GET /v1/audio/voices`; `kokoro-local` implements it by listing `voices_dir`;
`null` registers none — which is what makes `doctor` print no probe line for a
default project, behaviour a prior delivery had to fix once already and which
must survive this refactor.

**Concurrency is config, not capability.** `Backends` gains
`fan_out: BTreeMap<String, usize>`, populated per backend from its own settings
slice, defaulting to 1. `Backends::kokoro(id)` is then deleted, and with it the
last place where reaching a backend's capabilities required its concrete type.

## 7. Correctness

### 7.1 The style-index trap

Each voice file is `[510, 1, 256]` f32 indexed by **token count** —
`style[n_tokens]`, not a flat 256-vector. A wrong index produces speech with
subtly wrong prosody: plausible, and wrong, and cached, and published as
`measured`. Because the error only manifests at *some* token counts, one golden
fixture cannot be trusted to catch it.

This is now the **primary** correctness risk. Under the Burn design a parity
layer against ONNX Runtime existed partly to catch it; running *on* ONNX Runtime
removes that layer, so the tests below carry the whole weight.

### 7.2 Testing

1. **Golden fixtures across token lengths — offline, always on.** Not one
   utterance but several, chosen so their token counts differ: short, mid, and
   near the 510 ceiling. Generated once from the reference Python
   implementation by a committed script, so they are reproducible rather than
   magic. This replaces the Burn design's single fixture plus parity layer, and
   the multiple lengths are what make it equivalent.
2. **A style-index mutation test.** The suite must demonstrate that using
   `style[n+1]` makes a fixture fail. A tolerance that admits an off-by-one row
   is not a tolerance, and this is the only way to know which kind you have.
3. **Guards.** No NaN, non-silent, plausible duration.

Tolerance must be **derived from measured error and justified**, not guessed.

### 7.3 English only

`misaki-rs` covers US and British English. `capabilities().languages` must say
so with `LanguageSupport::Enumerated`, so a script naming another locale fails
at selection rather than producing mispronounced audio. HTTP Kokoro keeps the
rest.

### 7.4 Failure is never silence

Spec §7.1 applies unchanged. A missing model file, missing or unreadable
`voices_dir`, unknown voice, G2P failure, or inference error fails the command.
None fall back to `null`, to silence, or to another voice.

## 8. Risks

**A wrong or truncated model file.** Nothing verifies the `.onnx` against a
known digest — it is user-supplied at runtime, and pinning a digest would break
anyone using a quantized variant. Mitigation: the digest goes in the cache key
(§5.1), so a changed file changes the audio's identity rather than silently
reusing another model's cache entries. `doctor` reports the digest so a
mismatch between machines is visible.

**ONNX Runtime is a native dependency.** `ort`'s `download-binaries` fetches it
at build time; `load-dynamic` is the alternative for anyone who wants to supply
their own. This is the cost the design accepts, and §2 is the argument that it
is the cheapest one available.

**Quantized exports are unexplored.** The q8/q4/fp16 variants would cut the
311 MB substantially and probably the load time with it. They should work —
ONNX Runtime handles them natively, unlike a transpiler — but nobody has
checked, and the digest-in-cache-key design means switching is safe to try.

**CPU-bound work on an async trait.** ORT inference is synchronous; `dub` runs
on a current-thread runtime, so calling it directly would block the executor and
stall progress output and every sibling task. The backend wraps its session call
in `spawn_blocking`, which means this crate takes `tokio` with `rt`.

## 9. What breaks

- `Backends::kokoro(id)` is removed. Its three callers move to
  `registry.catalog(id)` (voice list, `origin()`) and `Backends::fan_out`.
- `VoiceRegistry` gains catalog registration. HTTP Kokoro's `voices()` moves
  from an inherent method to a trait impl and `base_url()` is exposed through
  `origin()`.
- `doctor` must still print **no probe line** for a `null`-backend project. That
  cost a fix round to get right; `null` registering no catalog preserves it, and
  a test must pin it on rendered output, not only JSON.
- Nothing in the artifact format changes.
- `teleprompt-compile`, `teleprompt-schedule` and `teleprompt-scene` stay
  untouched. §4.1's amended claim says the inner loop is where the original
  one-line promise held; if an implementer finds themselves editing any of those
  three, that is a finding to report, not a change to make quietly.

## 10. Delivery

**Not scheduled** — see [Disposition](#disposition). The breakdown below is
retained because it is the shape any revival would take.

No measurement gate this time — §2 already has the numbers, which is the whole
reason this document is about `ort` rather than Burn.

The crate and its feature gate; G2P and tokenization; the `VoiceCatalog`
refactor; the session module and backend; golden fixtures across token lengths
plus the mutation test; then docs.
