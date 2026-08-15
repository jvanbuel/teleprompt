# Narration Manifest Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `teleprompt dub`, which writes narration audio and a versioned JSON manifest so a video pipeline teleprompt does not control can consume teleprompt's dubbing.

**Architecture:** The `VoiceBackend` trait gains a second, opt-in call that returns PCM, leaving the existing timing-only `synthesize` untouched so `plan` and `diff` stay allocation-free. `teleprompt-compile` grows a `manifest` module that *joins* three sources — timings from the `Timeline`, text and chapters from the `Program`, word timings from the `SynthResult` — into a published document. The CLI writes one self-contained directory per locale.

**Tech Stack:** Rust 2021, no new dependencies. `serde`/`serde_json` for the manifest, `insta` for snapshots, a hand-written 16-bit PCM WAV encoder.

**Spec:** `docs/superpowers/specs/2026-08-15-teleprompt-narration-manifest-design.md` (referred to below as *the manifest spec*). It in turn depends on `docs/superpowers/specs/2026-08-15-teleprompt-design.md` (*the core design*); unqualified `§` references point at the core design, matching the manifest spec's own convention.

> **Amended before execution.** A pre-flight scan against the repository found
> six defects in this plan's first draft: a `parse` entry point that does not
> exist (the real one is `parse_script`, and it must be followed by
> `assign_ids`), chapters with no route from `Program` to the CLI, a
> `compile_script` extraction that duplicates one already in `cmd::check`, and
> Task 7/8 tests written against a `TempProject`/`run` harness that was never
> built. All are corrected above. The rulings are recorded in
> `.superpowers/sdd/2026-08-15-teleprompt-narration-manifest/progress.md`.

## Global Constraints

- MSRV is **1.75**. CI runs `cargo check` on 1.75; nothing may require a later feature. `u64::div_ceil` (1.73) is available; `Option::is_some_and` (1.70) is available.
- **No new dependencies**, workspace or crate. Everything here is buildable from `serde`, `serde_json`, `blake3`, `clap`, and `insta`, all already present.
- `cargo clippy --workspace --all-targets -- -D warnings` must pass. `cargo fmt --all --check` must pass.
- Every command's `--format json` output is a stable schema. Adding a field is allowed; renaming or removing one is a breaking change.
- Exit codes are fixed by §10.1 and centralised in `teleprompt_cli::output::exit_code_for`: `0` success, `1` runtime failure, `2` validation error, `3` drift, `4` voice downgrade. No subcommand invents a code.
- Output must be **deterministic**: identical inputs produce byte-identical manifests and byte-identical WAV files. No timestamps, no iteration over `HashMap`, no floats in serialized output.
- `MANIFEST_VERSION` is `1`. It is independent of `TIMELINE_VERSION`.
- Audio format in v1 is 16-bit PCM WAV at 48000 Hz, mono. `--format` accepts only `wav`.
- The test suite acquires no Node, no network, and no model dependency. The `null` backend must carry every test in this plan.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/teleprompt-voice/src/contract.rs` (modify) | Add `Pcm` and `VoiceBackend::render_pcm`. |
| `crates/teleprompt-voice/src/null.rs` (modify) | Return silence of exactly the estimated duration. |
| `crates/teleprompt-voice/src/wav.rs` (create) | Encode `Pcm` as a canonical RIFF/WAVE file. Nothing else. |
| `crates/teleprompt-core/src/program.rs` (modify) | Carry chapter provenance through `resolve`, which currently discards it. |
| `crates/teleprompt-compile/src/lib.rs` (modify) | Retain per-segment text, chapter, and word timings in `CompileOutput`. |
| `crates/teleprompt-compile/src/manifest.rs` (create) | The `NarrationManifest` types and the join that builds one. |
| `crates/teleprompt-compile/src/manifest_diff.rs` (create) | Compare two manifests; render the drift report. |
| `crates/teleprompt-cli/src/cmd/dub.rs` (create) | Orchestrate: compile, render audio, write the locale directory. |
| `crates/teleprompt-cli/src/main.rs` (modify) | The `Dub` subcommand and its exit-code wiring. |

`manifest.rs` and `manifest_diff.rs` are separate because the second is only reachable through `--check` and has its own rendering concerns; keeping them apart stops `manifest.rs` from becoming the module that does everything about manifests.

---

### Task 1: PCM in the voice contract

The existing `synthesize` returns timing only. `plan` and `diff` call it constantly and must not start allocating audio buffers, so audio arrives through a second method.

**Files:**
- Modify: `crates/teleprompt-voice/src/contract.rs`
- Modify: `crates/teleprompt-voice/src/null.rs`
- Modify: `crates/teleprompt-voice/src/lib.rs`
- Test: `crates/teleprompt-voice/tests/null.rs`

**Interfaces:**
- Consumes: `SynthRequest`, `SynthResult`, `VoiceError`, `VoiceBackend` from `crates/teleprompt-voice/src/contract.rs`.
- Produces:
  - `pub struct Pcm { pub sample_rate: u32, pub channels: u16, pub samples: Vec<i16> }`
  - `impl Pcm { pub fn duration_ms(&self) -> u64 }`
  - `fn VoiceBackend::render_pcm(&self, req: &SynthRequest) -> Result<Option<Pcm>, VoiceError>`
  - `pub const NULL_SAMPLE_RATE: u32 = 48_000;` in `null.rs`

- [ ] **Step 1: Write the failing test**

`crates/teleprompt-voice/tests/null.rs` already exists and already defines
`fn req(text: &str) -> SynthRequest` at line 3 and
`use teleprompt_voice::{NullVoice, SynthRequest, VoiceBackend};` at line 1.
**Append only the `#[test]` functions below and extend that existing `use`
line** with `Pcm` and `NULL_SAMPLE_RATE` — redefining `req` or the import is
a compile error.

Append to `crates/teleprompt-voice/tests/null.rs`:

```rust
// The existing line 1 becomes:
//   use teleprompt_voice::{NullVoice, Pcm, SynthRequest, VoiceBackend, NULL_SAMPLE_RATE};
// `req` is already defined in this file — do not redefine it.

#[test]
fn null_renders_silence_matching_its_own_estimate() {
    let v = NullVoice::default();
    let r = req("Every video in this repository is built from a script you can read.");

    let estimated = v.synthesize(&r).unwrap().duration_ms;
    let pcm = v.render_pcm(&r).unwrap().expect("null renders silence, not nothing");

    assert_eq!(pcm.sample_rate, NULL_SAMPLE_RATE);
    assert_eq!(pcm.channels, 1);
    assert!(
        pcm.samples.iter().all(|s| *s == 0),
        "null is silence, not noise"
    );
    assert_eq!(
        pcm.duration_ms(),
        estimated,
        "audio length must equal the duration the manifest will claim"
    );
}

#[test]
fn null_renders_nothing_for_empty_text() {
    let v = NullVoice::default();
    let pcm = v.render_pcm(&req("   ")).unwrap().unwrap();
    assert_eq!(pcm.samples.len(), 0);
    assert_eq!(pcm.duration_ms(), 0);
}

#[test]
fn null_rejects_a_non_positive_speed_when_rendering_too() {
    let v = NullVoice::default();
    let mut r = req("hello");
    r.speed = 0.0;
    assert!(v.render_pcm(&r).is_err());
}

#[test]
fn pcm_duration_is_computed_per_frame_not_per_sample() {
    let stereo = Pcm {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![0; 96_000], // 48_000 frames = 1000 ms
    };
    assert_eq!(stereo.duration_ms(), 1000);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p teleprompt-voice --test null`
Expected: FAIL to compile — `no method named render_pcm`, `cannot find type Pcm`, `cannot find value NULL_SAMPLE_RATE`.

- [ ] **Step 3: Add `Pcm` and the trait method**

In `crates/teleprompt-voice/src/contract.rs`, after `SynthResult`:

```rust
/// Interleaved 16-bit PCM. The one audio representation that crosses a
/// backend boundary — encoders live downstream of this type, not inside
/// backends, so every backend produces the same thing and only one place
/// knows about file formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl Pcm {
    /// Length in milliseconds, computed per *frame*: with two channels,
    /// two samples are one instant in time, not two.
    pub fn duration_ms(&self) -> u64 {
        let channels = self.channels.max(1) as u64;
        let rate = self.sample_rate.max(1) as u64;
        let frames = self.samples.len() as u64 / channels;
        frames * 1000 / rate
    }
}
```

Then add to the `VoiceBackend` trait, below `synthesize`:

```rust
    /// Render this request to audio. Separate from [`VoiceBackend::synthesize`]
    /// because the inner loop (`plan`, `diff`) needs durations thousands of
    /// times and audio never; making one call do both would put a
    /// multi-megabyte allocation on the hot path.
    ///
    /// Returns `Ok(None)` from a backend that genuinely cannot produce audio.
    /// `null` is not such a backend — it returns silence.
    fn render_pcm(&self, req: &SynthRequest) -> Result<Option<Pcm>, VoiceError>;
```

- [ ] **Step 4: Implement it for `NullVoice`**

In `crates/teleprompt-voice/src/null.rs`, add the constant near the other consts:

```rust
/// 48 kHz because it divides a millisecond exactly (48 samples), so a
/// duration in ms is always a whole number of frames and silence is never
/// a rounding away from the estimate it is supposed to match.
pub const NULL_SAMPLE_RATE: u32 = 48_000;
```

and add to `impl VoiceBackend for NullVoice`, after `synthesize`:

```rust
    fn render_pcm(&self, req: &SynthRequest) -> Result<Option<Pcm>, VoiceError> {
        let ms = self.synthesize(req)?.duration_ms;
        let frames = (ms * NULL_SAMPLE_RATE as u64).div_ceil(1000) as usize;
        Ok(Some(Pcm {
            sample_rate: NULL_SAMPLE_RATE,
            channels: 1,
            samples: vec![0; frames],
        }))
    }
```

Add `Pcm` to the `use crate::contract::{...}` list at the top of `null.rs`.

- [ ] **Step 5: Re-export from the crate root**

In `crates/teleprompt-voice/src/lib.rs`:

```rust
pub use contract::{
    LanguageSupport, Pcm, SynthRequest, SynthResult, VoiceBackend, VoiceCapabilities, VoiceError,
    WordTiming,
};
pub use null::{estimate_ms, NullVoice, NULL_SAMPLE_RATE};
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p teleprompt-voice`
Expected: PASS, including the four new tests.

Run: `cargo test --workspace`
Expected: PASS. Adding a required trait method breaks any other implementor; if the mock scene registry or a test double implements `VoiceBackend`, give it `Ok(None)` and a comment saying why.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-voice
git commit -m "feat(voice): render_pcm, so backends can emit audio without slowing plan"
```

---

### Task 2: WAV encoding

**Files:**
- Create: `crates/teleprompt-voice/src/wav.rs`
- Modify: `crates/teleprompt-voice/src/lib.rs`
- Test: `crates/teleprompt-voice/tests/wav.rs`

**Interfaces:**
- Consumes: `Pcm` from Task 1.
- Produces: `pub fn wav::encode(pcm: &Pcm) -> Vec<u8>`, re-exported as `teleprompt_voice::wav`.

- [ ] **Step 1: Write the failing test**

Create `crates/teleprompt-voice/tests/wav.rs`:

```rust
use teleprompt_voice::{wav, Pcm};

/// Three mono samples at 48 kHz. Every header field is hand-computed here
/// rather than read back through our own decoder, because a decoder that
/// shares the encoder's misunderstanding agrees with it perfectly.
#[test]
fn encodes_a_canonical_44_byte_header() {
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0, 1, -1],
    };
    let out = wav::encode(&pcm);

    assert_eq!(out.len(), 44 + 6, "44-byte header plus 3 samples of 2 bytes");

    assert_eq!(&out[0..4], b"RIFF");
    assert_eq!(&out[4..8], &42u32.to_le_bytes(), "36 + data length");
    assert_eq!(&out[8..12], b"WAVE");

    assert_eq!(&out[12..16], b"fmt ");
    assert_eq!(&out[16..20], &16u32.to_le_bytes(), "PCM fmt chunk is 16 bytes");
    assert_eq!(&out[20..22], &1u16.to_le_bytes(), "format tag 1 = PCM");
    assert_eq!(&out[22..24], &1u16.to_le_bytes(), "channels");
    assert_eq!(&out[24..28], &48_000u32.to_le_bytes(), "sample rate");
    assert_eq!(&out[28..32], &96_000u32.to_le_bytes(), "byte rate = rate * block align");
    assert_eq!(&out[32..34], &2u16.to_le_bytes(), "block align = channels * 2");
    assert_eq!(&out[34..36], &16u16.to_le_bytes(), "bits per sample");

    assert_eq!(&out[36..40], b"data");
    assert_eq!(&out[40..44], &6u32.to_le_bytes(), "data length");

    assert_eq!(&out[44..46], &0i16.to_le_bytes());
    assert_eq!(&out[46..48], &1i16.to_le_bytes());
    assert_eq!(&out[48..50], &(-1i16).to_le_bytes());
}

#[test]
fn encodes_stereo_block_alignment() {
    let pcm = Pcm {
        sample_rate: 44_100,
        channels: 2,
        samples: vec![0; 4],
    };
    let out = wav::encode(&pcm);
    assert_eq!(&out[22..24], &2u16.to_le_bytes(), "channels");
    assert_eq!(&out[32..34], &4u16.to_le_bytes(), "block align = 2 channels * 2 bytes");
    assert_eq!(&out[28..32], &176_400u32.to_le_bytes(), "44100 * 4");
}

#[test]
fn an_empty_pcm_is_a_valid_header_with_no_data() {
    let pcm = Pcm { sample_rate: 48_000, channels: 1, samples: vec![] };
    let out = wav::encode(&pcm);
    assert_eq!(out.len(), 44);
    assert_eq!(&out[40..44], &0u32.to_le_bytes());
    assert_eq!(&out[4..8], &36u32.to_le_bytes());
}

#[test]
fn encoding_is_byte_stable() {
    let pcm = Pcm { sample_rate: 48_000, channels: 1, samples: vec![0; 480] };
    assert_eq!(wav::encode(&pcm), wav::encode(&pcm));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p teleprompt-voice --test wav`
Expected: FAIL to compile — `could not find wav in teleprompt_voice`.

- [ ] **Step 3: Write the encoder**

Create `crates/teleprompt-voice/src/wav.rs`:

```rust
//! A 16-bit PCM RIFF/WAVE encoder, which is 30 lines and therefore not
//! worth a dependency. Output is byte-stable for byte-stable input, which
//! is what lets a committed narration directory be diffed.

use crate::contract::Pcm;

const HEADER_LEN: usize = 44;
const BITS_PER_SAMPLE: u16 = 16;

/// Encode `pcm` as a canonical WAVE file: one `fmt ` chunk, one `data`
/// chunk, no padding, little-endian throughout.
pub fn encode(pcm: &Pcm) -> Vec<u8> {
    let channels = pcm.channels;
    let block_align = channels * (BITS_PER_SAMPLE / 8);
    let byte_rate = pcm.sample_rate * block_align as u32;
    let data_len = (pcm.samples.len() * 2) as u32;

    let mut out = Vec::with_capacity(HEADER_LEN + data_len as usize);

    out.extend_from_slice(b"RIFF");
    // Everything after this field: 4 ("WAVE") + 24 (fmt chunk) + 8 (data
    // header) + the samples.
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&pcm.sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());

    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in &pcm.samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }

    out
}
```

- [ ] **Step 4: Declare and export the module**

In `crates/teleprompt-voice/src/lib.rs`, add `pub mod wav;` beside the other module declarations.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p teleprompt-voice --test wav`
Expected: PASS, 4 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/teleprompt-voice
git commit -m "feat(voice): 16-bit PCM WAV encoder"
```

---

### Task 3: Chapter provenance survives resolution

`resolve` flattens the chapter tree into a flat `Vec<Item>` and keeps nothing of it, so the manifest's `chapters` field is currently unreachable. Content before the first heading is already a hard error (`parse.rs:334`), so every narration item belongs to exactly one chapter and no `Option` is needed.

**Files:**
- Modify: `crates/teleprompt-core/src/program.rs`
- Modify: `crates/teleprompt-core/src/lib.rs`
- Test: `crates/teleprompt-core/tests/program.rs`

**Interfaces:**
- Consumes: `Script`, `Chapter` (fields `title`, `slug`) from `crates/teleprompt-core/src/ast.rs`.
- Produces:
  - `pub struct ChapterInfo { pub slug: String, pub title: String }`
  - `Program.chapters: Vec<ChapterInfo>` — document order, every chapter, including ones with no narration.
  - `Item::Narration.chapter: String` — the owning chapter's slug.

- [ ] **Step 1: Write the failing test**

**Shared fixture helper.** T3, T4, and T5 all need a resolved `Program`.
Define this once per test file — it is the exact front half of
`cmd::check::compile_script` (`check.rs:30-44`). `assign_ids` is not
optional: skip it and every segment id is the empty string, so every
downstream join is on `""`.

```rust
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Item, Program};

fn program_for(src: &str) -> Program {
    let mut parsed = parse_script(src).expect("fixture parses");
    let diags = assign_ids(&mut parsed);
    assert!(
        !diags.iter().any(|d| d.is_error()),
        "fixture must yield unambiguous segment ids"
    );
    resolve(
        &parsed,
        "tour.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect("fixture resolves")
}
```

With that in place, append to `crates/teleprompt-core/tests/program.rs`:

```rust
#[test]
fn resolve_records_chapters_in_document_order() {
    let src = "\
# Quick start

The first paragraph.

# Provenance

The second paragraph.
";
    let program = program_for(src);

    let chapters: Vec<(&str, &str)> = program
        .chapters
        .iter()
        .map(|c| (c.slug.as_str(), c.title.as_str()))
        .collect();
    assert_eq!(
        chapters,
        vec![("quick-start", "Quick start"), ("provenance", "Provenance")]
    );
}

#[test]
fn every_narration_names_its_owning_chapter() {
    let src = "\
# Quick start

The first paragraph.

# Provenance

The second paragraph.
";
    let program = program_for(src);

    let owners: Vec<&str> = program
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Narration { chapter, .. } => Some(chapter.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(owners, vec!["quick-start", "provenance"]);
}

#[test]
fn a_chapter_with_no_narration_is_still_recorded() {
    let src = "\
# Intro

Only this chapter speaks.

# Silent
";
    let program = program_for(src);

    assert_eq!(program.chapters.len(), 2, "resolve reports the script's shape, not just the spoken parts");
    assert_eq!(program.chapters[1].slug, "silent");
}
```

Match the existing imports at the top of that test file; add `ChapterInfo` and `Item` to them if they are not already there.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p teleprompt-core --test program`
Expected: FAIL to compile — `no field chapters on Program`, `struct Item::Narration has no field named chapter`.

- [ ] **Step 3: Add the types and fields**

In `crates/teleprompt-core/src/program.rs`, add above `Program`:

```rust
/// A chapter as the manifest and other consumers need it: identity and a
/// human title. `resolve` flattens chapters away, so without this the
/// script's structure would not survive compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterInfo {
    pub slug: String,
    pub title: String,
}
```

Add the field to `Program`:

```rust
pub struct Program {
    pub script_name: String,
    pub locale: String,
    pub config: Config,
    /// Every chapter in document order, whether or not it contains
    /// narration.
    pub chapters: Vec<ChapterInfo>,
    pub items: Vec<Item>,
}
```

Add the field to `Item::Narration`, after `source_hash`:

```rust
        /// Slug of the chapter this paragraph belongs to. Never empty:
        /// content before the first heading is rejected at parse time.
        chapter: String,
```

- [ ] **Step 4: Populate them in `resolve`**

In `resolve`, declare the accumulator next to `items`:

```rust
    let mut items = Vec::new();
    let mut chapters = Vec::new();
```

At the top of the `for chapter in &script.chapters` loop body, before the front-matter parse:

```rust
        chapters.push(ChapterInfo {
            slug: chapter.slug.clone(),
            title: chapter.title.clone(),
        });
```

In the `Node::Segment(seg)` arm, add the field to the pushed `Item::Narration`:

```rust
                    items.push(Item::Narration {
                        id: seg.id.clone().unwrap_or_default(),
                        source_hash: Hash::of(text.as_bytes()),
                        chapter: chapter.slug.clone(),
                        text,
                        config,
                        span: seg.span,
                    });
```

And in the returned `Program`:

```rust
    Ok(Program {
        script_name: script_name.to_string(),
        locale: locale.to_string(),
        config: base,
        chapters,
        items,
    })
```

- [ ] **Step 5: Export `ChapterInfo`**

In `crates/teleprompt-core/src/lib.rs`, add `ChapterInfo` wherever `Program` and `Item` are re-exported. If the module is exported wholesale as `pub mod program;` with no re-exports, leave it alone — downstream crates already reach it as `teleprompt_core::program::ChapterInfo`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p teleprompt-core`
Expected: PASS.

Run: `cargo test --workspace`
Expected: PASS. `teleprompt-compile` destructures `Item::Narration { .. }` with a rest pattern, so it needs no change; if any site enumerates fields exhaustively, add `chapter` there.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-core
git commit -m "feat(core): carry chapter provenance through resolve"
```

---

### Task 4: `CompileOutput` retains what the manifest needs

`compile` has the narration text, the chapter, and the backend's word timings in hand, and drops all three: the `Timeline` is a review surface and deliberately carries none of them. The manifest needs all three, so `compile` must stop discarding them.

**Files:**
- Modify: `crates/teleprompt-compile/src/lib.rs`
- Test: `crates/teleprompt-compile/tests/compile.rs`

**Interfaces:**
- Consumes: `Item::Narration.chapter` from Task 3. `WordTiming` from `teleprompt_voice`.
- Produces:
  - `pub struct NarrationDetail { pub segment_id: String, pub text: String, pub chapter: String, pub word_timings: Option<Vec<WordTiming>> }`
  - `CompileOutput.narration: Vec<NarrationDetail>` — one entry per narration item that synthesized successfully, in document order.
  - `CompileOutput.chapters: Vec<ChapterInfo>` — cloned from `Program.chapters`, document order, every chapter.

- [ ] **Step 1: Write the failing test**

Append to `crates/teleprompt-compile/tests/compile.rs`, matching that file's existing helpers for building a `Program` and calling `compile`:

```rust
#[test]
fn compile_retains_the_text_and_chapter_the_timeline_drops() {
    let src = "\
# Quick start

Every video here is built from a script.

# Provenance

And every timeline is committed.
";
    let out = compile_str(src).expect("compiles");

    let ids: Vec<&str> = out.narration.iter().map(|n| n.segment_id.as_str()).collect();
    assert_eq!(ids.len(), 2, "one detail per narration item, in document order");

    assert_eq!(
        out.narration[0].text,
        "Every video here is built from a script."
    );
    assert_eq!(out.narration[0].chapter, "quick-start");
    assert_eq!(out.narration[1].chapter, "provenance");

    assert!(
        out.narration[0].word_timings.is_none(),
        "the null backend advertises word_timings: false"
    );
}

#[test]
fn compile_carries_the_scripts_chapters() {
    let out = compile_str("\
# Quick start

The first paragraph.

# Provenance

The second paragraph.
").expect("compiles");

    let slugs: Vec<&str> = out.chapters.iter().map(|c| c.slug.as_str()).collect();
    assert_eq!(
        slugs,
        vec!["quick-start", "provenance"],
        "the CLI reaches compilation only through `compile_script`, which \
         returns this struct — chapters unreachable here are unreachable to `dub`"
    );
}

#[test]
fn narration_details_line_up_with_timeline_narration_entries() {
    let src = "\
# Quick start

The first paragraph here.

The second paragraph here.
";
    let out = compile_str(src).expect("compiles");

    let timeline_ids: Vec<&str> = out
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref().map(|n| n.segment.as_str()))
        .collect();
    let detail_ids: Vec<&str> = out.narration.iter().map(|n| n.segment_id.as_str()).collect();

    assert_eq!(
        timeline_ids, detail_ids,
        "the join key must be total in both directions, or the manifest \
         will silently drop or invent segments"
    );
}
```

Define `program_for` in this file exactly as Task 3's brief gives it, then add `compile_str` beside the other helpers:

```rust
fn compile_str(src: &str) -> Result<CompileOutput, Diagnostics> {
    let program = program_for(src);
    compile(
        &program,
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
        Path::new("."),
        "0.1.0",
    )
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p teleprompt-compile --test compile`
Expected: FAIL to compile — `no field narration on CompileOutput`.

- [ ] **Step 3: Add the type and the field**

In `crates/teleprompt-compile/src/lib.rs`, add above `CompileOutput`:

```rust
/// What the `Timeline` deliberately does not carry. The timeline is a
/// review surface — a reader scanning a pacing diff does not want the
/// prose inlined in it, and word timings would dwarf everything else. The
/// narration manifest needs all of it, so `compile` keeps it here rather
/// than making a second pass to recover what it already had.
#[derive(Debug, Clone)]
pub struct NarrationDetail {
    pub segment_id: String,
    pub text: String,
    pub chapter: String,
    pub word_timings: Option<Vec<WordTiming>>,
}
```

Extend `CompileOutput`:

```rust
#[derive(Debug)]
pub struct CompileOutput {
    pub timeline: Timeline,
    pub warnings: Vec<String>,
    /// One entry per narration item that synthesized, in document order.
    pub narration: Vec<NarrationDetail>,
    /// The script's chapters, in document order. Carried here because
    /// `compile` drops the `Program` and `cmd::check::compile_script` —
    /// the CLI's only route into compilation — returns just this struct.
    /// Without it the manifest's chapter markers are unreachable from the
    /// command that has to write them.
    pub chapters: Vec<ChapterInfo>,
}
```

Add `WordTiming` to the `use teleprompt_voice::{...}` import list.

- [ ] **Step 4: Populate it**

In `compile`, declare the accumulator beside `beats`:

```rust
    let mut narration_details: Vec<NarrationDetail> = Vec::new();
```

Destructure `chapter` in the `Item::Narration` arm's pattern:

```rust
            Item::Narration {
                id,
                text,
                source_hash,
                chapter,
                config,
                ..
            } => {
```

Immediately after the `synthesize` call succeeds and before `pending = Some(...)`, push the detail. It goes here, not earlier, because a segment whose synthesis failed has already `continue`d and must not appear in the manifest:

```rust
                narration_details.push(NarrationDetail {
                    segment_id: id.clone(),
                    text: text.clone(),
                    chapter: chapter.clone(),
                    word_timings: synth.word_timings.clone(),
                });
```

Return it:

```rust
    let (timeline, warnings) = schedule(&beats, &program.script_name, &program.locale, version);
    Ok(CompileOutput {
        timeline,
        warnings,
        narration: narration_details,
        chapters: program.chapters.clone(),
    })
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p teleprompt-compile`
Expected: PASS.

Run: `cargo test --workspace`
Expected: PASS. Any site constructing `CompileOutput` literally needs the new field; sites that only read `.timeline` are unaffected.

- [ ] **Step 6: Commit**

```bash
git add crates/teleprompt-compile
git commit -m "feat(compile): retain narration text, chapter, and word timings"
```

---

### Task 5: The manifest

**Files:**
- Create: `crates/teleprompt-compile/src/manifest.rs`
- Modify: `crates/teleprompt-compile/src/lib.rs` (add `pub mod manifest;`)
- Modify: `crates/teleprompt-compile/Cargo.toml` (add `serde.workspace = true`)
- Test: `crates/teleprompt-compile/tests/manifest.rs`

`teleprompt-compile` currently has `serde_json` only as a dev-dependency and no `serde`. Adding `serde` to its real dependencies is required and is not a new workspace dependency.

**Interfaces:**
- Consumes: `Timeline`, `NarrationEntry` from `teleprompt_schedule`; `ChapterInfo` from `teleprompt_core::program`; `NarrationDetail` from Task 4.
- Produces:
  - `pub const MANIFEST_VERSION: u32 = 1;`
  - `pub struct NarrationManifest { manifest_version, script, locale, generated_by, duration_ms, audio, chapters, segments }`
  - `pub struct AudioInfo { format: String, sample_rate: u32, channels: u16 }`
  - `pub struct ChapterEntry { id, title, start_ms }`
  - `pub struct SegmentEntry { id, text, start_ms, duration_ms, audio, voice_source, voice_source_actual, downgrade_reason, source_hash, audio_hash, words }`
  - `pub struct WordEntry { text, start_ms, end_ms }`
  - `pub fn build(timeline: &Timeline, chapters: &[ChapterInfo], details: &[NarrationDetail], audio: AudioInfo) -> NarrationManifest`
  - `pub fn audio_path(segment_id: &str, format: &str) -> String`

- [ ] **Step 1: Write the failing test**

Create `crates/teleprompt-compile/tests/manifest.rs`:

```rust
use std::path::Path;

use teleprompt_compile::manifest::{self, AudioInfo, MANIFEST_VERSION};
use teleprompt_compile::compile;
use teleprompt_core::config::PartialConfig;
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::NullVoice;

fn manifest_for(src: &str) -> manifest::NarrationManifest {
    let program = program_for(src);
    let out = compile(
        &program,
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
        Path::new("."),
        "0.1.0",
    )
    .unwrap();
    manifest::build(
        &out.timeline,
        &program.chapters,
        &out.narration,
        AudioInfo {
            format: "wav".to_string(),
            sample_rate: 48_000,
            channels: 1,
        },
    )
}

const TWO_CHAPTERS: &str = "\
# Quick start

Every video in this repository is built from a script you can read.

# Provenance

And every timeline is committed alongside it.
";

#[test]
fn segment_timings_equal_the_timeline_they_came_from() {
    let (out, m) = compiled_and_manifest(TWO_CHAPTERS);

    let from_timeline: Vec<(String, u64, u64)> = out
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .map(|n| (n.segment.clone(), n.start_ms, n.duration_ms))
        .collect();
    let from_manifest: Vec<(String, u64, u64)> = m
        .segments
        .iter()
        .map(|s| (s.id.clone(), s.start_ms, s.duration_ms))
        .collect();

    assert_eq!(
        from_timeline, from_manifest,
        "the manifest must not be able to disagree with the schedule it describes"
    );
    assert_eq!(m.duration_ms, out.timeline.duration_ms);
}

#[test]
fn chapters_carry_the_start_of_their_first_spoken_segment() {
    let m = manifest_for(TWO_CHAPTERS);
    assert_eq!(m.chapters.len(), 2);
    assert_eq!(m.chapters[0].id, "quick-start");
    assert_eq!(m.chapters[0].title, "Quick start");
    assert_eq!(m.chapters[0].start_ms, m.segments[0].start_ms);
    assert_eq!(m.chapters[1].id, "provenance");
    assert_eq!(m.chapters[1].start_ms, m.segments[1].start_ms);
}

#[test]
fn a_chapter_with_no_narration_is_omitted() {
    let m = manifest_for("\
# Intro

Only this chapter speaks.

# Silent
");
    let ids: Vec<&str> = m.chapters.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["intro"],
        "a chapter marker with no start time would be meaningless to a consumer"
    );
}

#[test]
fn audio_paths_are_relative_to_the_manifest() {
    let m = manifest_for(TWO_CHAPTERS);
    assert_eq!(m.segments[0].audio, format!("audio/{}.wav", m.segments[0].id));
    assert!(
        !m.segments[0].audio.starts_with('/'),
        "an absolute path would not survive being served from anywhere else"
    );
}

#[test]
fn header_records_version_provenance_and_audio_shape() {
    let m = manifest_for(TWO_CHAPTERS);
    assert_eq!(m.manifest_version, MANIFEST_VERSION);
    assert_eq!(m.script, "tour.md");
    assert_eq!(m.locale, "en");
    assert_eq!(m.generated_by, "teleprompt 0.1.0");
    assert_eq!(m.audio.format, "wav");
    assert_eq!(m.audio.sample_rate, 48_000);
    assert_eq!(m.audio.channels, 1);
}

#[test]
fn downgrade_reason_is_present_as_null_but_words_are_omitted() {
    let m = manifest_for(TWO_CHAPTERS);
    let json = serde_json::to_value(&m).unwrap();
    let seg = &json["segments"][0];

    assert!(
        seg.get("downgrade_reason").is_some(),
        "consumers should not have to distinguish absent from null here"
    );
    assert!(seg["downgrade_reason"].is_null());
    assert!(
        seg.get("words").is_none(),
        "absent means the backend has no word timings; an empty array would be a lie"
    );
}

#[test]
fn the_manifest_json_shape_is_stable() {
    let m = manifest_for(TWO_CHAPTERS);
    insta::assert_json_snapshot!(m);
}

#[test]
fn serialization_is_byte_stable() {
    let a = serde_json::to_string_pretty(&manifest_for(TWO_CHAPTERS)).unwrap();
    let b = serde_json::to_string_pretty(&manifest_for(TWO_CHAPTERS)).unwrap();
    assert_eq!(a, b);
}

```

Add `insta.workspace = true` to `teleprompt-compile`'s `[dev-dependencies]`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p teleprompt-compile --test manifest`
Expected: FAIL to compile — `could not find manifest in teleprompt_compile`.

- [ ] **Step 3: Write the manifest module**

Create `crates/teleprompt-compile/src/manifest.rs`:

```rust
//! The narration manifest: teleprompt's published output for pipelines that
//! render themselves.
//!
//! This is a **join**, not a projection. Timings come from the [`Timeline`],
//! but `text` and `chapters` live in the `Program` and `words` lives in the
//! `SynthResult` the scheduler discards. This crate is the only one holding
//! all three, which is why the type lives here rather than beside
//! `Timeline`.

use serde::{Deserialize, Serialize};
use teleprompt_core::program::ChapterInfo;
use teleprompt_core::Hash;
use teleprompt_schedule::Timeline;

use crate::NarrationDetail;

/// Incremented on any breaking change to the shape below. Independent of
/// `TIMELINE_VERSION`: the timeline is an internal review surface, the
/// manifest is a contract with third parties, and they will not move
/// together.
pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NarrationManifest {
    pub manifest_version: u32,
    pub script: String,
    pub locale: String,
    pub generated_by: String,
    pub duration_ms: u64,
    pub audio: AudioInfo,
    pub chapters: Vec<ChapterEntry>,
    pub segments: Vec<SegmentEntry>,
}

/// Uniform across every segment, so a consumer configures its player once
/// rather than probing each file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioInfo {
    pub format: String,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterEntry {
    pub id: String,
    pub title: String,
    pub start_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentEntry {
    pub id: String,
    pub text: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub audio: String,
    pub voice_source: String,
    pub voice_source_actual: String,
    /// Always serialized. `null` when the requested tier was delivered —
    /// a consumer should not have to tell "absent" from "no downgrade".
    pub downgrade_reason: Option<String>,
    pub source_hash: Hash,
    pub audio_hash: Hash,
    /// Omitted entirely when the backend has no word timings, so absence is
    /// unambiguous and never confused with an empty list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<WordEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordEntry {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Where a segment's audio sits, relative to the manifest that names it.
pub fn audio_path(segment_id: &str, format: &str) -> String {
    format!("audio/{segment_id}.{format}")
}

/// Join a scheduled [`Timeline`] with the narration detail `compile`
/// retained, producing the published manifest.
///
/// A `NarrationDetail` with no matching timeline entry is dropped, and a
/// timeline narration entry with no matching detail is dropped: both are
/// impossible for output from a single `compile` call, and neither is worth
/// a fallible signature that every caller would then `unwrap`.
pub fn build(
    timeline: &Timeline,
    chapters: &[ChapterInfo],
    details: &[NarrationDetail],
    audio: AudioInfo,
) -> NarrationManifest {
    let segments: Vec<SegmentEntry> = timeline
        .entries
        .iter()
        .filter_map(|entry| {
            let n = entry.narration.as_ref()?;
            let detail = details.iter().find(|d| d.segment_id == n.segment)?;
            Some(SegmentEntry {
                id: n.segment.clone(),
                text: detail.text.clone(),
                start_ms: n.start_ms,
                duration_ms: n.duration_ms,
                audio: audio_path(&n.segment, &audio.format),
                voice_source: n.voice_source.clone(),
                voice_source_actual: n.voice_source_actual.clone(),
                downgrade_reason: n.downgrade_reason.clone(),
                source_hash: n.source_hash,
                audio_hash: n.audio_hash,
                words: detail.word_timings.as_ref().map(|ws| {
                    ws.iter()
                        .map(|w| WordEntry {
                            text: w.word.clone(),
                            start_ms: w.start_ms,
                            end_ms: w.end_ms,
                        })
                        .collect()
                }),
            })
        })
        .collect();

    // A chapter's start is its first spoken segment's start. A chapter with
    // nothing spoken in it has no defensible time and is omitted rather
    // than given a guessed one.
    let chapter_entries = chapters
        .iter()
        .filter_map(|c| {
            let start_ms = details
                .iter()
                .filter(|d| d.chapter == c.slug)
                .filter_map(|d| segments.iter().find(|s| s.id == d.segment_id))
                .map(|s| s.start_ms)
                .min()?;
            Some(ChapterEntry {
                id: c.slug.clone(),
                title: c.title.clone(),
                start_ms,
            })
        })
        .collect();

    NarrationManifest {
        manifest_version: MANIFEST_VERSION,
        script: timeline.script.clone(),
        locale: timeline.locale.clone(),
        generated_by: timeline.generated_by.clone(),
        duration_ms: timeline.duration_ms,
        audio,
        chapters: chapter_entries,
        segments,
    }
}
```

- [ ] **Step 4: Wire the module and the dependency**

In `crates/teleprompt-compile/src/lib.rs`, add near the top:

```rust
pub mod manifest;
```

In `crates/teleprompt-compile/Cargo.toml`:

```toml
[dependencies]
teleprompt-core = { path = "../teleprompt-core" }
teleprompt-scene = { path = "../teleprompt-scene" }
teleprompt-schedule = { path = "../teleprompt-schedule" }
teleprompt-voice = { path = "../teleprompt-voice" }
serde.workspace = true

[dev-dependencies]
insta.workspace = true
serde_json.workspace = true
```

- [ ] **Step 5: Run the tests and accept the snapshot**

Run: `cargo test -p teleprompt-compile --test manifest`
Expected: FAIL once on the snapshot test with a pending `.snap.new` file.

Run: `cargo insta accept` (or review and rename the `.snap.new` file by hand).

Run: `cargo test -p teleprompt-compile --test manifest`
Expected: PASS, 9 tests.

Read the accepted snapshot before committing it. Confirm by eye: `manifest_version` is 1, `downgrade_reason` appears as `null`, no `words` key appears, and every `audio` path is `audio/<id>.wav`.

- [ ] **Step 6: Commit**

```bash
git add crates/teleprompt-compile
git commit -m "feat(compile): the narration manifest"
```

---

### Task 6: Manifest drift

`--check` must fail a pull request whose committed manifest no longer matches its script, exactly as `diff --exit-code` does for the timeline. The report distinguishes an author's edit from an audio-only change, because they call for different fixes.

**Files:**
- Create: `crates/teleprompt-compile/src/manifest_diff.rs`
- Modify: `crates/teleprompt-compile/src/lib.rs` (add `pub mod manifest_diff;`)
- Test: `crates/teleprompt-compile/tests/manifest_diff.rs`

**Interfaces:**
- Consumes: `NarrationManifest`, `SegmentEntry` from Task 5.
- Produces:
  - `pub struct ManifestDiff { pub duration_before_ms, pub duration_after_ms, pub added: Vec<String>, pub removed: Vec<String>, pub changed: Vec<ChangedSegment>, pub reordered: bool }`
  - `pub struct ChangedSegment { pub id: String, pub before_ms: u64, pub after_ms: u64, pub reason: String }`
  - `pub fn diff(before: &NarrationManifest, after: &NarrationManifest) -> ManifestDiff`
  - `impl ManifestDiff { pub fn is_empty(&self) -> bool; pub fn render(&self) -> String }`

- [ ] **Step 1: Write the failing test**

Create `crates/teleprompt-compile/tests/manifest_diff.rs`:

```rust
use teleprompt_compile::manifest::{AudioInfo, NarrationManifest, SegmentEntry, MANIFEST_VERSION};
use teleprompt_compile::manifest_diff::diff;
use teleprompt_core::Hash;

fn seg(id: &str, start_ms: u64, duration_ms: u64, text: &str, audio_seed: &str) -> SegmentEntry {
    SegmentEntry {
        id: id.to_string(),
        text: text.to_string(),
        start_ms,
        duration_ms,
        audio: format!("audio/{id}.wav"),
        voice_source: "synthetic".to_string(),
        voice_source_actual: "synthetic".to_string(),
        downgrade_reason: None,
        source_hash: Hash::of(text.as_bytes()),
        audio_hash: Hash::of(audio_seed.as_bytes()),
        words: None,
    }
}

fn manifest(segments: Vec<SegmentEntry>) -> NarrationManifest {
    let duration_ms = segments.last().map(|s| s.start_ms + s.duration_ms).unwrap_or(0);
    NarrationManifest {
        manifest_version: MANIFEST_VERSION,
        script: "tour.md".to_string(),
        locale: "en".to_string(),
        generated_by: "teleprompt 0.1.0".to_string(),
        duration_ms,
        audio: AudioInfo { format: "wav".into(), sample_rate: 48_000, channels: 1 },
        chapters: Vec::new(),
        segments,
    }
}

#[test]
fn an_unchanged_manifest_reports_nothing() {
    let m = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let d = diff(&m, &m);
    assert!(d.is_empty());
}

#[test]
fn an_edited_paragraph_is_reported_as_a_text_edit() {
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![seg("welcome", 0, 2000, "Hello there, at length.", "a")]);
    let d = diff(&before, &after);

    assert!(!d.is_empty());
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].id, "welcome");
    assert_eq!(d.changed[0].before_ms, 1000);
    assert_eq!(d.changed[0].after_ms, 2000);
    assert_eq!(d.changed[0].reason, "text edited");
}

#[test]
fn a_new_voice_is_reported_as_an_audio_change_not_a_text_edit() {
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![seg("welcome", 0, 1100, "Hello.", "b")]);
    let d = diff(&before, &after);

    assert_eq!(d.changed[0].reason, "audio changed");
}

#[test]
fn a_segment_that_only_moved_is_still_drift() {
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![seg("welcome", 500, 1000, "Hello.", "a")]);
    let d = diff(&before, &after);

    assert!(!d.is_empty(), "a moved segment desynchronises every consumer");
    assert_eq!(d.changed[0].reason, "moved");
}

#[test]
fn added_and_removed_segments_are_named() {
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![
        seg("welcome", 0, 1000, "Hello.", "a"),
        seg("outro", 1000, 500, "Bye.", "c"),
    ]);
    let d = diff(&before, &after);
    assert_eq!(d.added, vec!["outro"]);
    assert!(d.removed.is_empty());

    let back = diff(&after, &before);
    assert_eq!(back.removed, vec!["outro"]);
    assert!(back.added.is_empty());
}

#[test]
fn reordering_two_unchanged_segments_is_drift() {
    let a = seg("one", 0, 1000, "First.", "a");
    let mut b = seg("two", 1000, 1000, "Second.", "b");
    let before = manifest(vec![a.clone(), b.clone()]);

    b.start_ms = 0;
    let mut a2 = a.clone();
    a2.start_ms = 1000;
    let after = manifest(vec![b, a2]);

    let d = diff(&before, &after);
    assert!(d.reordered, "order is part of the contract, not an accident");
    assert!(!d.is_empty());
}

#[test]
fn the_report_names_the_duration_change_and_the_segments() {
    let before = manifest(vec![seg("welcome", 0, 1000, "Hello.", "a")]);
    let after = manifest(vec![seg("welcome", 0, 2000, "Hello there, at length.", "a")]);
    let text = diff(&before, &after).render();

    assert!(text.contains("1.0s"), "{text}");
    assert!(text.contains("2.0s"), "{text}");
    assert!(text.contains("welcome"), "{text}");
    assert!(text.contains("text edited"), "{text}");
    assert!(text.contains("needs re-render"), "{text}");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p teleprompt-compile --test manifest_diff`
Expected: FAIL to compile — `could not find manifest_diff in teleprompt_compile`.

- [ ] **Step 3: Write the module**

Create `crates/teleprompt-compile/src/manifest_diff.rs`:

```rust
//! Drift detection for a committed narration manifest.
//!
//! This is what keeps teleprompt's central claim working when the picture
//! is rendered by something teleprompt does not control: a pull request
//! that edits prose without regenerating fails, exactly as one with a
//! stale timeline does.

use serde::Serialize;

use crate::manifest::{NarrationManifest, SegmentEntry};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangedSegment {
    pub id: String,
    pub before_ms: u64,
    pub after_ms: u64,
    /// `text edited` | `audio changed` | `moved`. Different causes want
    /// different fixes, so the report must not collapse them.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestDiff {
    pub duration_before_ms: u64,
    pub duration_after_ms: u64,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<ChangedSegment>,
    pub reordered: bool,
}

impl ManifestDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && !self.reordered
            && self.duration_before_ms == self.duration_after_ms
    }

    pub fn render(&self) -> String {
        if self.is_empty() {
            return "no narration changes\n".to_string();
        }

        let mut out = String::new();
        let delta = self.duration_after_ms as i64 - self.duration_before_ms as i64;
        out.push_str(&format!(
            "narration: {} → {} ({}{})\n",
            secs(self.duration_before_ms),
            secs(self.duration_after_ms),
            if delta >= 0 { "+" } else { "-" },
            secs(delta.unsigned_abs()),
        ));

        if !self.added.is_empty() {
            out.push_str("\nadded:\n");
            for id in &self.added {
                out.push_str(&format!("  {id}\n"));
            }
        }
        if !self.removed.is_empty() {
            out.push_str("\nremoved:\n");
            for id in &self.removed {
                out.push_str(&format!("  {id}\n"));
            }
        }
        if !self.changed.is_empty() {
            out.push_str("\nchanged:\n");
            for c in &self.changed {
                out.push_str(&format!(
                    "  {:<16} {} → {}  ({})\n",
                    c.id,
                    secs(c.before_ms),
                    secs(c.after_ms),
                    c.reason
                ));
            }
        }
        if self.reordered {
            out.push_str("\nreordered: segment order changed\n");
        }

        let mut stale: Vec<&str> = self.changed.iter().map(|c| c.id.as_str()).collect();
        stale.extend(self.added.iter().map(String::as_str));
        if !stale.is_empty() {
            out.push_str("\nneeds re-render:\n");
            for id in stale {
                out.push_str(&format!("  {id}\n"));
            }
        }

        out
    }
}

fn secs(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

/// Why a segment differs, in the order that makes the report most useful:
/// a text edit is the author's own doing and explains everything
/// downstream, so it is named even when the audio and position also moved.
fn reason_for(before: &SegmentEntry, after: &SegmentEntry) -> Option<String> {
    if before.source_hash != after.source_hash {
        Some("text edited".to_string())
    } else if before.audio_hash != after.audio_hash {
        Some("audio changed".to_string())
    } else if before.start_ms != after.start_ms || before.duration_ms != after.duration_ms {
        Some("moved".to_string())
    } else if before.voice_source_actual != after.voice_source_actual {
        Some(format!(
            "voice tier {} → {}",
            before.voice_source_actual, after.voice_source_actual
        ))
    } else {
        None
    }
}

pub fn diff(before: &NarrationManifest, after: &NarrationManifest) -> ManifestDiff {
    let added = after
        .segments
        .iter()
        .filter(|s| !before.segments.iter().any(|b| b.id == s.id))
        .map(|s| s.id.clone())
        .collect();
    let removed = before
        .segments
        .iter()
        .filter(|s| !after.segments.iter().any(|a| a.id == s.id))
        .map(|s| s.id.clone())
        .collect();

    let changed = before
        .segments
        .iter()
        .filter_map(|b| {
            let a = after.segments.iter().find(|a| a.id == b.id)?;
            reason_for(b, a).map(|reason| ChangedSegment {
                id: b.id.clone(),
                before_ms: b.duration_ms,
                after_ms: a.duration_ms,
                reason,
            })
        })
        .collect();

    // Order is part of the contract: a consumer iterating `segments` builds
    // its own sequence from them, so two identical segments that swapped
    // places are drift even though neither one changed.
    let before_order: Vec<&str> = before.segments.iter().map(|s| s.id.as_str()).collect();
    let after_order: Vec<&str> = after.segments.iter().map(|s| s.id.as_str()).collect();
    let common_before: Vec<&str> = before_order
        .iter()
        .copied()
        .filter(|id| after_order.contains(id))
        .collect();
    let common_after: Vec<&str> = after_order
        .iter()
        .copied()
        .filter(|id| before_order.contains(id))
        .collect();

    ManifestDiff {
        duration_before_ms: before.duration_ms,
        duration_after_ms: after.duration_ms,
        added,
        removed,
        changed,
        reordered: common_before != common_after,
    }
}
```

- [ ] **Step 4: Declare the module**

In `crates/teleprompt-compile/src/lib.rs`, beside `pub mod manifest;`:

```rust
pub mod manifest_diff;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p teleprompt-compile --test manifest_diff`
Expected: PASS, 7 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/teleprompt-compile
git commit -m "feat(compile): narration manifest drift detection"
```

---

### Task 7: `teleprompt dub`

**Files:**
- Create: `crates/teleprompt-cli/src/cmd/dub.rs`
- Modify: `crates/teleprompt-cli/src/cmd/mod.rs`
- Modify: `crates/teleprompt-cli/src/main.rs`
- Test: `crates/teleprompt-cli/tests/dub.rs`

Read `crates/teleprompt-cli/src/cmd/plan.rs` first: `run_plan` is the closest existing shape, and `dub` should resolve its project, script, and locale exactly the way it does rather than inventing a second convention.

**Interfaces:**
- Consumes: `Project::for_script`, `manifest::build`, `manifest::AudioInfo`, `manifest_diff::diff`, `wav::encode`, `VoiceBackend::render_pcm`, `NULL_SAMPLE_RATE`.
- Produces:
  - `pub struct DubOutput { pub manifest: NarrationManifest, pub written: Vec<PathBuf>, pub warnings: Vec<String>, pub drift: Option<ManifestDiff> }`
  - `pub fn run_dub(project: &Project, script: &Path, locale: &str, out_root: &Path, check_only: bool) -> Result<DubOutput, Vec<String>>`

- [ ] **Step 1: Write the failing test**

Create `crates/teleprompt-cli/tests/dub.rs`. It follows the idiom already in
`crates/teleprompt-cli/tests/commands.rs`: `project_with` scaffolds a real
project in a temp directory, and binary-level behaviour (exit codes, stdout)
is asserted by running `env!("CARGO_BIN_EXE_teleprompt")`. Copy the two
helpers below rather than importing them — `commands.rs` keeps its helpers
private and this plan does not restructure it.

```rust
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use teleprompt_cli::project::Project;

const SCRIPT: &str = "\
# Quick start

Every video in this repository is built from a script you can read.

# Provenance

And every timeline is committed alongside it.
";

fn tempdir(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "teleprompt-dub-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

/// Scaffold a project and drop `script` at `scripts/test.md`, mirroring
/// `commands.rs::project_with`. Returns the project root.
fn project_with(tag: &str, script: &str) -> PathBuf {
    let dir = tempdir(tag);
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(dir.join("scripts/test.md"), script).unwrap();
    Project::discover(&dir).unwrap();
    dir
}

fn tp(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("process exited normally")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn read_manifest(root: &Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(root.join("public/narration/en/narration.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn dub_writes_a_manifest_and_one_wav_per_segment() {
    let root = project_with("write", SCRIPT);
    let out = tp(&root, &["dub", "scripts/test.md", "--out", "public/narration"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    let m = read_manifest(&root);
    assert_eq!(m["manifest_version"], 1);
    assert_eq!(m["locale"], "en");
    assert_eq!(m["segments"].as_array().unwrap().len(), 2);

    for seg in m["segments"].as_array().unwrap() {
        let rel = seg["audio"].as_str().unwrap();
        let wav = root.join("public/narration/en").join(rel);
        assert!(wav.exists(), "missing {rel}");
        assert_eq!(&std::fs::read(&wav).unwrap()[0..4], b"RIFF");
    }
}

#[test]
fn the_wav_length_matches_the_duration_the_manifest_claims() {
    let root = project_with("length", SCRIPT);
    tp(&root, &["dub", "scripts/test.md", "--out", "public/narration"]);

    let m = read_manifest(&root);
    let seg = &m["segments"][0];
    let claimed_ms = seg["duration_ms"].as_u64().unwrap();

    let bytes = std::fs::read(
        root.join("public/narration/en").join(seg["audio"].as_str().unwrap()),
    )
    .unwrap();
    let data_len = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as u64;
    let actual_ms = data_len / 2 * 1000 / 48_000;

    assert_eq!(
        actual_ms, claimed_ms,
        "a consumer placing this file at its stated duration must not clip it"
    );
}

#[test]
fn dub_is_byte_stable_across_runs() {
    let root = project_with("stable", SCRIPT);
    tp(&root, &["dub", "scripts/test.md", "--out", "a"]);
    tp(&root, &["dub", "scripts/test.md", "--out", "b"]);

    let a = std::fs::read(root.join("a/en/narration.json")).unwrap();
    let b = std::fs::read(root.join("b/en/narration.json")).unwrap();
    assert_eq!(a, b, "a committed manifest must not churn between runs");
}

#[test]
fn check_passes_on_a_fresh_manifest_and_writes_nothing() {
    let root = project_with("checkclean", SCRIPT);
    tp(&root, &["dub", "scripts/test.md", "--out", "public/narration"]);

    let before = std::fs::read(root.join("public/narration/en/narration.json")).unwrap();
    let out = tp(&root, &["dub", "scripts/test.md", "--out", "public/narration", "--check"]);
    let after = std::fs::read(root.join("public/narration/en/narration.json")).unwrap();

    assert_eq!(code(&out), 0, "{}", stdout(&out));
    assert_eq!(before, after, "--check must not write");
}

#[test]
fn check_exits_3_when_the_prose_moved_on() {
    let root = project_with("drift", SCRIPT);
    tp(&root, &["dub", "scripts/test.md", "--out", "public/narration"]);

    std::fs::write(
        root.join("scripts/test.md"),
        SCRIPT.replace(
            "And every timeline is committed alongside it.",
            "And every timeline is committed alongside it, which is what makes \
             the whole review story work at all.",
        ),
    )
    .unwrap();

    let out = tp(&root, &["dub", "scripts/test.md", "--out", "public/narration", "--check"]);
    assert_eq!(code(&out), 3, "stale manifest must fail CI");
    assert!(stdout(&out).contains("text edited"), "{}", stdout(&out));
}

#[test]
fn check_exits_3_when_no_manifest_has_been_written_at_all() {
    let root = project_with("nomanifest", SCRIPT);
    let out = tp(&root, &["dub", "scripts/test.md", "--out", "public/narration", "--check"]);
    assert_eq!(code(&out), 3, "a missing manifest is maximal drift, not a crash");
}

#[test]
fn a_manifest_from_a_future_version_is_refused_rather_than_misread() {
    let root = project_with("future", SCRIPT);
    tp(&root, &["dub", "scripts/test.md", "--out", "public/narration"]);

    let path = root.join("public/narration/en/narration.json");
    let raw = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        raw.replace("\"manifest_version\": 1", "\"manifest_version\": 999"),
    )
    .unwrap();

    let out = tp(&root, &["dub", "scripts/test.md", "--out", "public/narration", "--check"]);
    assert_eq!(code(&out), 1, "an unreadable manifest is a runtime failure, not drift");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("manifest_version"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_broken_script_fails_validation_before_writing_anything() {
    let root = project_with("broken", "# Intro\n\nOne. {#a polcy=hold}\n");
    let out = tp(&root, &["dub", "scripts/test.md", "--out", "public/narration"]);

    assert_eq!(code(&out), 2);
    assert!(
        !root.join("public/narration/en").exists(),
        "no partial output"
    );
}
```

`teleprompt-cli` already has `serde_json` as a dependency, so the test file
needs no manifest change.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p teleprompt-cli --test dub`
Expected: FAIL — `unrecognized subcommand 'dub'`.

- [ ] **Step 3: Write the command**

Create `crates/teleprompt-cli/src/cmd/dub.rs`:

```rust
use std::path::{Path, PathBuf};

use teleprompt_compile::manifest::{self, AudioInfo, NarrationManifest, MANIFEST_VERSION};
use teleprompt_compile::manifest_diff::{self, ManifestDiff};
use teleprompt_voice::{wav, NullVoice, VoiceBackend};

use crate::project::Project;

pub struct DubOutput {
    pub manifest: NarrationManifest,
    pub written: Vec<PathBuf>,
    pub warnings: Vec<String>,
    /// `Some` only under `--check`. `None` means nothing was compared.
    pub drift: Option<ManifestDiff>,
}

/// Where this locale's self-contained directory lives.
fn locale_dir(out_root: &Path, locale: &str) -> PathBuf {
    out_root.join(locale)
}

pub fn manifest_path(out_root: &Path, locale: &str) -> PathBuf {
    locale_dir(out_root, locale).join("narration.json")
}

/// Read a committed manifest, refusing a version this build does not
/// understand rather than letting serde fill in defaults and produce a
/// confident, wrong diff.
fn read_committed(path: &Path) -> Result<Option<NarrationManifest>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let probe: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let version = probe.get("manifest_version").and_then(|v| v.as_u64());
    if version != Some(MANIFEST_VERSION as u64) {
        return Err(format!(
            "{}: manifest_version {} is not supported (this build writes {})",
            path.display(),
            version.map(|v| v.to_string()).unwrap_or_else(|| "missing".into()),
            MANIFEST_VERSION,
        ));
    }
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

pub fn run_dub(
    project: &Project,
    script: &Path,
    locale: &str,
    out_root: &Path,
    check_only: bool,
) -> Result<DubOutput, Vec<String>> {
    let voice = NullVoice::default();
    let compiled = crate::cmd::plan::compile_script(project, script, locale, &voice)?;

    // Audio is rendered before anything is written, so a synthesis failure
    // cannot leave a half-populated directory behind.
    let mut audio: Vec<(String, Vec<u8>)> = Vec::new();
    let mut sample_rate = teleprompt_voice::NULL_SAMPLE_RATE;
    let mut channels = 1u16;

    for detail in &compiled.narration {
        let req = teleprompt_voice::SynthRequest {
            text: detail.text.clone(),
            locale: locale.to_string(),
            voice: None,
            speed: 1.0,
        };
        match voice.render_pcm(&req) {
            Ok(Some(pcm)) => {
                sample_rate = pcm.sample_rate;
                channels = pcm.channels;
                audio.push((detail.segment_id.clone(), wav::encode(&pcm)));
            }
            Ok(None) => {
                return Err(vec![format!(
                    "backend `{}` produces no audio; `dub` needs a backend that can render",
                    voice.id()
                )])
            }
            Err(e) => return Err(vec![format!("segment `{}`: {e}", detail.segment_id)]),
        }
    }

    let built = manifest::build(
        &compiled.timeline,
        &compiled.chapters,
        &compiled.narration,
        AudioInfo {
            format: "wav".to_string(),
            sample_rate,
            channels,
        },
    );

    if check_only {
        let committed = read_committed(&manifest_path(out_root, locale)).map_err(|e| vec![e])?;
        let drift = match committed {
            Some(before) => manifest_diff::diff(&before, &built),
            // No manifest at all is maximal drift: everything is new.
            None => manifest_diff::diff(
                &NarrationManifest {
                    segments: Vec::new(),
                    chapters: Vec::new(),
                    duration_ms: 0,
                    ..built.clone()
                },
                &built,
            ),
        };
        return Ok(DubOutput {
            manifest: built,
            written: Vec::new(),
            warnings: compiled.warnings,
            drift: Some(drift),
        });
    }

    let dir = locale_dir(out_root, locale);
    let audio_dir = dir.join("audio");
    std::fs::create_dir_all(&audio_dir)
        .map_err(|e| vec![format!("cannot create {}: {e}", audio_dir.display())])?;

    let mut written = Vec::new();
    for (segment_id, bytes) in &audio {
        let path = dir.join(manifest::audio_path(segment_id, "wav"));
        std::fs::write(&path, bytes)
            .map_err(|e| vec![format!("cannot write {}: {e}", path.display())])?;
        written.push(path);
    }

    let path = manifest_path(out_root, locale);
    let json = serde_json::to_string_pretty(&built)
        .map_err(|e| vec![format!("cannot serialize manifest: {e}")])?;
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| vec![format!("cannot write {}: {e}", path.display())])?;
    written.push(path);

    Ok(DubOutput {
        manifest: built,
        written,
        warnings: compiled.warnings,
        drift: None,
    })
}

pub fn render_dub(out: &DubOutput) -> String {
    let mut s = format!(
        "{} ({}) — {} segment(s), {:.1}s\n",
        out.manifest.script,
        out.manifest.locale,
        out.manifest.segments.len(),
        out.manifest.duration_ms as f64 / 1000.0,
    );
    for path in &out.written {
        s.push_str(&format!("  wrote {}\n", path.display()));
    }
    s
}
```

`run_dub` calls `crate::cmd::check::compile_script`, which already exists
with signature `(project, script, locale) -> Result<CompileOutput, Vec<String>>`
and already constructs the `NullVoice` and the `SceneRegistry` internally.
**Do not extract a new one and do not change its signature** — Task 4 put
`chapters` on `CompileOutput`, so everything `dub` needs already comes back
from the existing call, and `run_check`, `run_plan`, and `run_diff` stay
untouched.

That means `run_dub` does not construct its own backend for compilation. It
does need one for `render_pcm`, and `NullVoice::default()` is the only
backend that exists; when a real backend lands, both call sites select it
together. Note the duplication in the report so the reviewer sees it was
deliberate.

- [ ] **Step 4: Register the module and the subcommand**

In `crates/teleprompt-cli/src/cmd/mod.rs`, add `pub mod dub;`.

In `crates/teleprompt-cli/src/main.rs`, add to `enum Command`:

```rust
    /// Synthesize narration and write audio plus a manifest
    Dub {
        script: PathBuf,
        #[arg(long, default_value = "en")]
        locale: String,
        /// Output root; one self-contained directory is written per locale
        #[arg(long)]
        out: PathBuf,
        /// Compare against the manifest on disk and write nothing; exit 3 on drift
        #[arg(long)]
        check: bool,
    },
```

and the match arm, following the shape of the `Diff` arm above it:

```rust
        Command::Dub {
            script,
            locale,
            out,
            check,
        } => match Project::for_script(&script) {
            Err(e) => {
                eprintln!("error: {e}");
                Outcome::RuntimeFailure(e.to_string())
            }
            Ok(project) => match dub::run_dub(&project, &script, &locale, &out, check) {
                Ok(result) => {
                    for w in &result.warnings {
                        eprintln!("warning: {w}");
                    }
                    match (&result.drift, cli.format) {
                        (Some(d), Format::Json) => {
                            println!("{}", serde_json::to_string_pretty(d).unwrap())
                        }
                        (Some(d), Format::Human) => print!("{}", d.render()),
                        (None, Format::Json) => {
                            println!("{}", serde_json::to_string_pretty(&result.manifest).unwrap())
                        }
                        (None, Format::Human) => print!("{}", dub::render_dub(&result)),
                    }
                    match &result.drift {
                        Some(d) if !d.is_empty() => Outcome::Drift,
                        _ => Outcome::Ok,
                    }
                }
                Err(errors) => {
                    match cli.format {
                        Format::Json => {
                            let report = ErrorReport::new(errors.clone());
                            println!("{}", serde_json::to_string_pretty(&report).unwrap())
                        }
                        Format::Human => {
                            for e in &errors {
                                eprintln!("{e}");
                            }
                        }
                    }
                    Outcome::ValidationError(errors)
                }
            },
        },
```

Add `use teleprompt_cli::cmd::dub;` to the imports.

Note the one place this arm departs from `Diff`: a manifest that cannot be read is a *runtime* failure, not a validation error. `run_dub` returns those through the same `Err` channel, so map them by checking whether the error came from `read_committed` — simplest is for `run_dub` to return a typed error enum with `Validation(Vec<String>)` and `Runtime(String)` variants. Do that rather than pattern-matching on message text.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p teleprompt-cli --test dub`
Expected: PASS, 8 tests.

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/teleprompt-cli
git commit -m "feat(cli): teleprompt dub"
```

---

### Task 8: End-to-end acceptance, doctor, and documentation

The manifest is a published contract, so the acceptance test asserts the exact document a consumer will receive, and the README carries the consumer-side code that the spec's §7 describes.

**Files:**
- Create: `crates/teleprompt-cli/tests/dub_acceptance.rs`
- Modify: `crates/teleprompt-cli/src/cmd/doctor.rs`
- Modify: `crates/teleprompt-cli/Cargo.toml` (add `insta` as a dev-dependency if absent)
- Modify: `README.md`
- Test: the acceptance test below, plus a snapshot

**Interfaces:**
- Consumes: everything above. Produces nothing new.

- [ ] **Step 1: Write the failing acceptance test**

Create `crates/teleprompt-cli/tests/dub_acceptance.rs` rather than appending
to `end_to_end.rs`: these tests need the `tp` / `project_with` helpers from
Task 7's `dub.rs`, and `end_to_end.rs` has its own fixture conventions.
Copy the same four helpers (`tempdir`, `project_with`, `tp`, `code`) that
Task 7's brief gives.

```rust
// helpers: same tempdir / project_with / tp / code as tests/dub.rs

const TOUR: &str = include_str!("../../../tests/fixtures/tour.md");

#[test]
fn dub_produces_the_document_a_consumer_will_read() {
    let root = project_with("acceptance", TOUR);
    let out = tp(&root, &["dub", "scripts/test.md", "--out", "public/narration"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    let raw = std::fs::read_to_string(root.join("public/narration/en/narration.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&raw).unwrap();
    insta::assert_json_snapshot!(manifest);
}

#[test]
fn the_committed_manifest_gates_ci_the_way_the_timeline_does() {
    let root = project_with("gate", TOUR);
    tp(&root, &["dub", "scripts/test.md", "--out", "public/narration"]);

    assert_eq!(
        code(&tp(&root, &["dub", "scripts/test.md", "--out", "public/narration", "--check"])),
        0,
        "clean tree passes"
    );

    std::fs::write(
        root.join("scripts/test.md"),
        format!("{TOUR}\nOne more sentence, added late.\n"),
    )
    .unwrap();

    assert_eq!(
        code(&tp(&root, &["dub", "scripts/test.md", "--out", "public/narration", "--check"])),
        3,
        "an edit that was never re-dubbed must fail the build"
    );
}
```

`tests/fixtures/tour.md` sits at the repository root, so from
`crates/teleprompt-cli/tests/` the `include_str!` path is three levels up.
Add `insta.workspace = true` to `teleprompt-cli`'s `[dev-dependencies]` if
it is not already there.

- [ ] **Step 2: Run to verify it fails, then accept the snapshot**

Run: `cargo test -p teleprompt-cli --test dub_acceptance`
Expected: FAIL on the pending snapshot.

Run: `cargo insta accept`

Read the accepted snapshot in full before committing. It is the contract; if anything in it would embarrass you in a consumer's `console.log`, fix it now rather than after `manifest_version` has to be bumped.

- [ ] **Step 3: Report the manifest in `doctor`**

In `crates/teleprompt-cli/src/cmd/doctor.rs`, add a line reporting what `dub` can produce, matching the existing report's shape:

```rust
    lines.push(("manifest".to_string(), format!("narration v{}", MANIFEST_VERSION)));
    lines.push((
        "note".to_string(),
        "dub writes 16-bit PCM WAV at 48 kHz; the null backend renders silence \
         of the estimated duration."
            .to_string(),
    ));
```

Adapt the field names to whatever `DoctorReport` actually uses; do not restructure it.

- [ ] **Step 4: Run the doctor test**

Run: `cargo test -p teleprompt-cli`
Expected: PASS. Update any doctor snapshot or assertion that enumerates the report's lines.

- [ ] **Step 5: Document the consumer side**

Add to `README.md`, after the existing command overview:

````markdown
## Dubbing a pipeline that renders itself

`teleprompt dub` writes narration audio and a manifest, and renders no video.
A tool teleprompt does not control — Remotion, After Effects, a web player —
reads the manifest and owns its own picture.

```bash
teleprompt dub scripts/tour.md --out public/narration
```

```
public/narration/en/narration.json
public/narration/en/audio/welcome.wav
```

The manifest gives every segment an id, its prose, an absolute `start_ms` and
`duration_ms`, the audio path, and which voice tier actually produced it.
Deriving your composition's length from `duration_ms` is what keeps
teleprompt's central property working on the other side of the boundary:
narration length still drives pacing.

### Consuming it from Remotion

```tsx
const msToFrames = (ms: number, fps: number) => Math.round((ms * fps) / 1000);

calculateMetadata={async ({props}) => {
  const m = await (await fetch(staticFile(props.manifestPath))).json();
  if (m.manifest_version !== 1) throw new Error(`unsupported manifest ${m.manifest_version}`);
  return {durationInFrames: msToFrames(m.duration_ms, 30), props: {...props, manifest: m}};
}}
```

Convert **absolute offsets** to frames and take the difference for a
segment's length — never round a duration on its own, or the rounding error
accumulates and drifts audio out of sync by the end of a long video:

```tsx
const from = msToFrames(seg.start_ms, fps);
const until = msToFrames(next ? next.start_ms : m.duration_ms, fps);
// durationInFrames={until - from}
```

### Keeping it honest

```bash
teleprompt dub scripts/tour.md --out public/narration --check
```

Exits `3` when the committed manifest no longer matches the script, so a pull
request that edits prose without re-dubbing fails CI. Commit
`narration.json`; `audio/` is reproducible and can be gitignored, at the cost
of needing a voice backend wherever you render.

teleprompt ships no Remotion code and takes no Remotion dependency. Remotion's
own licence — free for individuals and organisations up to three employees —
is between you and Remotion.
````

- [ ] **Step 6: Full verification**

Run: `cargo test --workspace`
Expected: PASS.

Run: `cargo fmt --all --check`
Expected: clean.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

Run the loop by hand once, and read the output rather than only checking the exit code:

```bash
cargo run -- dub tests/fixtures/tour.md --out /tmp/tp-dub
cat /tmp/tp-dub/en/narration.json
cargo run -- dub tests/fixtures/tour.md --out /tmp/tp-dub --check ; echo "exit=$?"
```

Expected: a manifest whose segment timings match `cargo run -- plan tests/fixtures/tour.md`, WAV files that play as silence of the right length, and `exit=0` on the second command.

- [ ] **Step 7: Commit**

```bash
git add crates/teleprompt-cli README.md
git commit -m "test(cli): end-to-end dub acceptance; document the consumer side"
```

---

## Self-Review

**Spec coverage.** Every section of the manifest spec maps to a task: §3 `dub` and its flags → Tasks 7, 1, 2; §4 output layout → Task 7; §4.1 commit guidance → Task 8 (README); §5 the manifest and every field in §5.1 → Task 5; §5.2 determinism → Tasks 2, 5, 7; §5.3 `--check` → Tasks 6, 7; §6 licensing → Task 8 (README); §7 worked example and §7.1 the rounding rule → Task 8; §8 no npm package → nothing to build; §9 deferrals → nothing to build; §10 crate structure → Tasks 3–7, testing → all tasks; §11 risks → the drift test in Task 5 and the join-totality test in Task 4.

Two spec requirements have no task and are deliberate: `--chapter` and repeatable `--locale`/`all` from §3. Both are listed in the spec's command synopsis but exercise no new machinery — they are argument plumbing over a loop, and `plan` does not implement `--chapter` yet either. Adding them here would put `dub` ahead of the rest of the CLI on a dimension that should move together. Filed as a follow-up rather than silently dropped: **`dub` in this plan handles one locale per invocation and always the whole script.** The `--locale` flag exists and takes one value; the directory layout already anticipates several.

**Placeholder scan.** No TBD, no "handle errors appropriately", no "similar to Task N". Every code step carries the code. Task 7 Step 3 and Task 8 Step 3 both instruct the implementer to adapt to an existing shape (`plan.rs`'s helper, `DoctorReport`'s fields) rather than showing it, because those shapes are in the repository and inventing a second one in the plan would be worse than reading the first.

**Type consistency.** `Pcm` (Task 1) is consumed by `wav::encode` (Task 2) and `run_dub` (Task 7). `ChapterInfo` (Task 3) is consumed by `manifest::build` (Task 5) and threaded through `compile_script` (Task 7). `NarrationDetail` (Task 4) is consumed by `manifest::build` (Task 5). `NarrationManifest` and `SegmentEntry` (Task 5) are consumed by `manifest_diff` (Task 6) and `run_dub` (Task 7). `MANIFEST_VERSION` is defined once in Task 5 and read in Tasks 7 and 8. `WordTiming.word` is the field name in `teleprompt-voice`; the manifest's `WordEntry.text` renames it deliberately, and Task 5's `build` performs that rename explicitly.

One consistency risk worth naming for the executor: Task 7's `run_dub` needs both a validation-error and a runtime-error channel, and Task 7 Step 4 says so but Step 3's signature returns `Result<DubOutput, Vec<String>>`. **Resolve this in favour of the typed enum** — introduce `pub enum DubError { Validation(Vec<String>), Runtime(String) }` in Step 3 and have Step 4's match arm use it. The flat `Vec<String>` in Step 3's sketch is the thing to change, not the requirement in Step 4.
