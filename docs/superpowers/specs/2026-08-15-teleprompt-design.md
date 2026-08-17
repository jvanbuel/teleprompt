# teleprompt — Design Specification

**Status:** approved design, pre-implementation
**Date:** 2026-08-15

## 1. Summary

teleprompt compiles videos from version-controlled source. A Markdown script
describes what the viewer sees and what the narrator says; teleprompt turns it
into a narrated, optionally multi-language video.

The point is the feedback loop. Editing a paragraph changes narration duration,
which shifts every transition after it. teleprompt makes that consequence
visible and cheap: `plan` computes the new timeline without rendering a frame,
`diff` explains what moved, and `build` re-renders only the parts that changed.

The Playwright analogy is more than an analogy: browser scenes *are* Playwright
scripts. A script is a program, execution is reproducible, and the artifact
under version control is the script rather than the recording.

Scripts need not be written from scratch. The expected origin is that an author
records themselves doing the thing, and teleprompt derives the script from what
it observed (§8) — after which the same feedback loop applies.

### Goals

- A video is a deterministic function of a committed script plus a cache.
- Narration duration is the primary input to visual pacing.
- Narration ranges from stock TTS to a clone of the author's voice to the
  author reading live, mixable within one script.
- One script produces N locale variants, each with its own correct timing.
- Every generated artifact is content-addressed, so edits cost the minimum
  possible recomputation.
- Scenes reuse the language that already owns their domain rather than inventing
  one.

### Non-goals (v1)

- A timeline editor. teleprompt is a compiler; editing happens in the script.
- A step language of teleprompt's own. Adapters wrap existing tools (§7).
- Motion-graphics authoring. Transitions are chosen, not designed.
- Translation itself. teleprompt maintains the ledger, not the prose.
- Hosting, publishing, or analytics.
- Real-time streaming. Output is a file.

## 2. Concepts

| Term | Meaning |
|---|---|
| **Script** | The Markdown source file. The unit of compilation. |
| **Chapter** | A Markdown heading and everything under it. Addressable build target. |
| **Segment** | One narration paragraph. Carries a stable ID. The atom of voice and translation. |
| **Action block** | A fenced block of adapter-native source. Executed by one adapter. |
| **Scene** | A domain the video can show: `browser`, `terminal`, `media`. Names the *what*. |
| **Adapter** | The implementation serving a scene — `playwright`, `vhs` — wrapping an existing tool behind the `Scene` contract. Names the *how*. |
| **Mark** | A yield point inside an action block where the scheduler regains control. Beat boundary. |
| **Beat** | A segment plus the action span that follows it. The scheduling unit. |
| **Program** | A script resolved for one locale: config merged, translations applied. |
| **Timeline** | Compiled schedule — absolute times for every clip and span. Committed. |
| **Take** | One human reading of one segment: a file, or a slice of a longer recording. |
| **Trace** | Structured record of observed actions with timestamps, produced by recording a live session. |
| **Voice source** | Which tier on the dubbing spectrum produced a segment's audio. |

## 3. Artifact format

### 3.1 Shape

Front matter configures. Prose paragraphs are segments. Fenced `teleprompt`
blocks are action blocks, written in whatever language their scene speaks (§7).
Headings are chapters.

````markdown
---
teleprompt: 1
locales:
  source: en
  targets: [nl, fr]
voice:
  source: synthetic                                  # synthetic | cloned | recorded
  synthetic: { backend: kokoro, model: af_heart, speed: 1.0 }
  cloned:    { backend: elevenlabs, profile: jan }   # see voices/jan.toml
  recorded:  { takes_dir: takes }
scene:
  default: browser
  browser:
    base_url: "http://localhost:3000"
    viewport: [1920, 1080]
    device_scale_factor: 2
output:
  resolution: [1920, 1080]
  fps: 30
  transition: { kind: crossfade, duration: auto, max_ms: 600 }
---

# Quickstart

Welcome to Acme. Let me show you how to get a project running. {#welcome}

```teleprompt scene=browser
await page.goto('/');
await page.getByText('Get started').waitFor();
```

Click through here, and you land on the project wizard.

```teleprompt scene=browser policy=concurrent align=start
await page.getByText('Get started').click();
await page.waitForURL('/new');
```

Give it a name, and you are done. {#done voice.source=recorded}

```teleprompt scene=browser policy=stretch-action
await tp.type('input[name=title]', 'My first project');
await page.getByText('Create').click();
```

And here is the same thing from the command line. {#cli}

```teleprompt scene=terminal
Type "acme new my-project"
Enter
Sleep 2s
```
````

### 3.2 Parsing rules

- The document is parsed as CommonMark. Only three node kinds are meaningful:
  headings (chapters), paragraphs (segments), and fenced code blocks with the
  `teleprompt` info string (action blocks).
- Any other node — lists, tables, block quotes, images, other code fences — is
  **ignored for compilation** and is available for authoring notes. This lets a
  script double as human-readable documentation.
- A paragraph consisting only of an HTML comment is a directive, not narration:
  `<!-- teleprompt: pause 800ms -->`.
- Inline text is normalised for speech: emphasis markers stripped, inline code
  spans read as their contents, links read as their text.
- **An action block's body is opaque to the Markdown parser.** teleprompt reads
  the fence attributes to learn which scene owns the block, then hands the body
  to that scene's adapter for validation. teleprompt itself never interprets the
  contents.
- `include=<path>` on the fence replaces the body with the contents of an
  external file, so a Playwright script can live in a real `.spec.ts` that
  editors and Playwright's own tooling understand. A fence may have a body or an
  `include`, not both.

### 3.3 Segment identity

Segment IDs are the anchor for caching, translation, and take binding. They must
be stable across unrelated edits.

- An explicit trailing `{#id}` wins.
- Otherwise the ID is derived: `<chapter-slug>-<n>` where `n` is the segment's
  ordinal within its chapter (`quickstart-1`, `quickstart-2`).

Derived IDs are positional and therefore shift when a segment is inserted.
`teleprompt check` warns when a build would reassign a derived ID that has
cached audio or a recorded take attached, and `teleprompt loc sync --promote`
rewrites derived IDs into explicit `{#id}` anchors in place. Scripts with
recorded takes should promote their IDs; this is documented as the recommended
practice and enforced by a warning, not a hard error.

### 3.4 Attributes

Attributes attach to segments in the `{#id key=value}` suffix and to action
blocks in the fence info string. Both use the same `key=value` grammar. Known
keys are validated at parse time; unknown keys are an error, not a silent
ignore.

Segment keys: `voice.source`, `voice.backend`, `voice.voice`, `voice.speed`,
`lead_in`, `tail`, `lang` (override for a single segment, e.g. a product name
that should keep source pronunciation).

Action block keys: `scene`, `include`, `policy`, `align`, `id`, `max_speedup`
(bounds `trim-action`), `max_stretch` and `min_stretch` (bound
`stretch-action`).

Fence attributes are teleprompt's, parsed by teleprompt. Everything inside the
fence belongs to the scene. That line is what keeps adapters independent: adding
a scene never changes the Markdown grammar.

### 3.5 Configuration resolution

Configuration merges in this order, last wins:

1. Built-in defaults.
2. `teleprompt.toml` at the project root (shared across scripts).
3. Script front matter.
4. Chapter-level front matter (a YAML block immediately after a heading).
5. Segment or action-block attributes.
6. CLI flags.

Secrets never appear in any of these. API credentials come from the environment
(`ELEVENLABS_API_KEY`) or an OS keyring entry referenced by name. `check`
verifies presence without printing values.

### 3.6 Localization

The source-locale script is the sole home of action blocks and structure.
Translations are sidecar files containing only narration:

```markdown
<!-- script.nl.md -->
---
teleprompt-locale: nl
of: script.md
---

## welcome {source_hash=8f2a1c}
Welkom bij Acme. Ik laat je zien hoe je een project opstart.

## done {source_hash=b41e77}
Geef het een naam, en je bent klaar.
```

A sidecar is parsed with its own grammar, distinct from a source script: here a
heading names a **segment ID**, not a chapter, and no action blocks or
directives are permitted. The `of:` key binds the sidecar to its source, and
`teleprompt-locale` distinguishes the two file kinds unambiguously at parse
time.

Each translated segment records the `source_hash` it was translated from. When
the source paragraph changes, its hash changes and the translation is **stale** —
surfaced by `loc status`, reported by `diff`, and warned about (not fatal) by
`build`.

`loc sync` reconciles: it adds sections for new segments, marks stale ones, and
flags orphans whose source segment disappeared. It never deletes translated
text; orphans are moved to a commented block at the end of the file so nothing
is lost to a rename.

Translation itself is out of scope. teleprompt maintains the ledger; a human or
an external tool fills in the text.

## 4. The dubbing spectrum

Three tiers on one axis — how much of the author is in the sound.

| tier | source | re-render cost | cross-locale |
|---|---|---|---|
| `synthetic` | stock TTS voice | free, instant | any language the backend supports |
| `cloned` | TTS driven by a profile enrolled from the author's recordings | instant, per-call API cost | cross-lingual: the author's voice, another language |
| `recorded` | the author reading the prompter live | a human must redo it | source locale, realistically |

Declared as `voice.source` at any configuration level, so tiers mix within one
script: record the introduction, let TTS carry the reference material.

### 4.1 Fallback ladder

`recorded → cloned → synthetic`.

When a tier cannot produce audio for a segment — no take recorded, take stale,
no voice profile enrolled, no API key — the build **drops one tier and
continues**, recording the substitution in the timeline as
`voice_source_actual` alongside the requested `voice_source`. A fresh clone of
the repository therefore always produces a watchable video, and the timeline
always states which parts are not really the author yet.

`build --strict-voice` makes any downgrade fatal, for release builds and CI.

### 4.2 Take staleness

A take is a **slice of an audio file**, not necessarily a whole one:

```rust
pub struct Take {
    pub file: PathBuf,
    pub offset_ms: u64,
    pub duration_ms: u64,
    pub source_hash: Hash,      // segment text this take was read from
    pub recorded_at: Timestamp,
}
```

The slice is essential rather than cosmetic. Per-segment recording produces
whole files, but the record-first flow (§8) produces one continuous take
spanning an entire script, sliced into segments after transcription. Both must
be the same type, or every consumer — cache key, staleness check, ffmpeg
placement, take management — needs two code paths. A whole-file take is simply
`offset_ms: 0` with the file's full duration.

A take binds to the `source_hash` of the segment that produced it. Editing the
paragraph invalidates the take.

This is the one place in the loop with irreducible friction: synthesized audio
regenerates for free, a human take does not. The design principle is to make
that friction visible rather than silently ship audio that no longer matches the
script:

- `diff` lists stale takes explicitly, separately from other changes.
- `build` degrades a tier and emits a warning per stale segment.
- `record --stale` records exactly the drifted segments, in script order.

The hash binding is implemented from M0, well before recording exists.
Retrofitting identity and hashing onto an existing cache is far more expensive
than carrying it from the start.

### 4.3 Backend capabilities

Backends advertise what they can do; `check` fails early when a script asks for
something the configured backend cannot deliver, rather than failing mid-build
after other work has been paid for.

```rust
pub struct VoiceCapabilities {
    pub languages: LanguageSupport,   // Any | Enumerated(Vec<LanguageTag>)
    pub cloning: bool,
    pub cross_lingual: bool,
    pub word_timings: bool,
    pub ssml: bool,
    pub speed_control: bool,
}
```

v1 backends:

- **`null`** — the backend itself synthesizes silence of the estimated
  length. The words-per-minute duration model behind that estimate,
  punctuation-aware, lives in `teleprompt-voice-null` as `WpmEstimator`, the
  default implementation of `DurationEstimator` — the same trait `plan`,
  `check`, and `diff` call to predict a duration without synthesizing.
  No audio quality, but exact enough timing to iterate on pacing at zero
  cost. Also makes the entire scheduler test suite hermetic and offline. It
  is a first-class backend, not a test double.
- **`kokoro`** — local, open-weight, fast, no cloning. Invoked as a subprocess
  against a local server rather than linked in-process, so a model-runtime
  problem cannot take down the CLI.
- **`elevenlabs`** — HTTP. Provides cloning and cross-lingual synthesis; the v1
  path to the `cloned` tier.
- **`recorded`** — not a synthesizer. Resolves a segment to a take file,
  validating the hash binding. Implements the same interface so the pipeline has
  one code path.

A local clone-capable backend (XTTS-v2 / F5-TTS class) is a later addition
behind the same trait; nothing in the design assumes cloning implies a network
call.

### 4.4 Enrollment

```
teleprompt voice enroll --name jan --from "takes/**/*.wav" [--backend elevenlabs]
```

Builds a voice profile from existing audio and writes a reference — provider ID,
enrollment source hashes, created-at — to `voices/jan.toml`, which is committed.
The audio itself is not. Enrollment is a deliberate, explicitly-invoked act:
teleprompt never uploads recordings as a side effect of a build.

### 4.5 Recording as a pipeline stage

Recording occupies exactly the position synthesis does — before scheduling — so
the audio-first architecture is unchanged. Only the origin of the duration
differs.

`teleprompt record` presents a prompter: the segment text, scrolling at a pace
derived from the estimated duration (or, when recording a target locale, from
the source-locale take being matched), with a pace indicator showing drift
against that target. It records the microphone, writes the audio under `takes/`,
and registers a `Take` in `takes/index.toml` — the manifest binding segment IDs
to file, offset, duration, and `source_hash`. Multiple takes per segment are
kept; the newest valid one wins unless the manifest marks a pick.

The manifest, not the directory layout, is the source of truth. That is what
lets a single continuous recording serve many segments without copying audio.

The take's *actual* duration is what enters the timeline. The loop is: plan with
estimates, record, re-plan with truth.

## 5. Compilation pipeline

```
parse ─► resolve ─► voice ─► schedule ─► capture ─► compose
```

1. **Parse** — Markdown to AST. Assign segment IDs, validate attributes, hand
   each action block to its adapter for validation and mark-splitting. Pure
   apart from the adapter's own validation. This is where `check` stops.
2. **Resolve** — merge configuration, apply the locale overlay, resolve the
   voice source per segment. Produces a `Program`.
3. **Voice** — for each segment, obtain an `AudioClip { path, duration,
   word_timings }` by synthesis or take lookup. Content-addressed; unchanged
   segments cost nothing.
4. **Schedule** — apply beat policies, compute the `Timeline`.
5. **Capture** — run each scene against its timeline slice, producing video
   segments. Only beats whose inputs changed are re-captured.
6. **Compose** — build the ffmpeg graph, mux, emit the video and subtitle
   sidecars.

Stages 1–4 are the *plan*; stages 5–6 are the *render*. `plan` runs 1–4 and
stops, which is what makes the inner loop fast: with the `null` backend, a full
re-plan of a long script is a sub-second, fully offline operation.

**Measurement.** Adapters that cannot estimate a span statically (§7.2) need
their spans timed once. The measuring pass sits between stages 4 and 5, runs
only for spans with no cached measurement, and writes results to the cache keyed
on span content hash. A first build therefore pays a dry-run; subsequent plans
are as fast as the fully-static case. Measurement never blocks `plan`, which
reports unmeasured spans as estimates and marks them as such in the timeline.

### 5.1 Caching

One content-addressed store under `.teleprompt/cache/`, keyed by BLAKE3 of a
canonical description of the inputs:

| artifact | key inputs |
|---|---|
| audio clip | segment text, locale, backend ID, voice, speed, backend version |
| span measurement | span source, scene kind, scene config, adapter version |
| video segment | span source, scene kind, viewport, scene config, slot duration |
| composed output | timeline hash, all member artifact hashes, output settings |

Cache entries are immutable and safe to delete at any time; deleting one costs
recomputation, never correctness. `cache ls` reports size and age per class;
`cache clean --unreferenced` prunes what no committed timeline points at.

Two properties keep the store honest. Backend version participates in the key,
so upgrading a TTS model invalidates its clips rather than silently mixing
voices. Slot duration participates in the video key, so a beat re-captures when
narration length changes even though its steps did not.

## 6. Scheduling

The scheduler is the heart of the tool and the most heavily tested component. It
is a pure function: `(Program, Vec<AudioClip>, Vec<SceneEstimate>) -> Timeline`.
No IO, no clocks, fully deterministic.

### 6.1 Beats

A beat is a segment plus the action block immediately following it. Either half
may be absent: a segment with no action block is narration over a held visual; an
action block with no segment is silent business.

### 6.2 Policies

**No policy ever alters the narration.** Every policy below takes the
narration's duration as a fixed target and adjusts only the action — the
typing speed, the scroll rate, the progress animation, the machine-generated
motion that has no natural tempo of its own. Speech is always played at the
length the voice backend produced it, and the scheduler has no capability to
resample, compress, or truncate it. When the pacing is wrong, the remedy is
editing the prose and re-running the loop, not stretching the waveform.

`voice.speed` is the one speech-rate control in the system, and it sits
outside this table on purpose: it is an author-set synthesis parameter passed
to the backend in `SynthRequest`, so the voice is *generated* at that rate
rather than resampled after the fact. The scheduler must never acquire the
ability to write it in order to make a beat fit.

The two policies that adjust something are named for what they adjust:
`stretch-action` and `trim-action`. An earlier draft called them `stretch` and
`trim`, which named an operation without naming its object and read as if the
speech were being adjusted — the first question a reader asked of this document
was whether teleprompt time-stretches speech. `check` recognises the old
spellings and reports the new ones (issue #1).

| policy | semantics |
|---|---|
| `hold` (default) | Narration plays over the visual state left by the previous beat. The action runs after narration completes. Predictable, and correct for most explanatory content. |
| `concurrent` | Action and narration overlap. `align=start` (default) begins them together; `align=end` makes them finish together; `align=center` centres the shorter within the longer. Whichever ends first waits. |
| `stretch-action` | The action's internal delays scale by a uniform factor so the block's duration exactly equals the narration's. Intended for typing, scrolling, and progress animations. Bounded by `max_stretch` (default 3.0) and `min_stretch` (default 0.33); exceeding either is a warning and the bound is applied. |
| `trim-action` | An action longer than its narration is time-compressed in post until it fits, bounded by `max_speedup` (default 2.0). Beyond the bound, the action's tail is cut at the last completed step and a warning is emitted. The narration sets the length and is itself untouched — an action shorter than its narration is left alone. |

### 6.3 Slack and automatic transitions

The slack of a beat is the difference between its narration duration and its
action duration. This is the mechanism by which text length informs the visuals.

With `transition.duration: auto`, the transition into the next beat is
`clamp(slack * 0.5, min_ms, max_ms)`, then **capped at the quiet window** —
generous pauses get a visible crossfade, tight ones get a hard cut. Authors who
want fixed pacing set an explicit duration; the automatic mode exists so that
the default behaviour of editing prose is that the video's rhythm follows the
writing.

**The quiet window.** A transition overlaps the beat it leaves, so an
uncapped one eats into whatever is still playing there. The quiet window is the
time around the boundary when nobody is speaking: this beat's trailing silence
(from where its narration ends to where the beat ends, normally `tail`) plus
the next beat's `lead_in`. An `auto` transition never exceeds it.

Without the cap, a script of plain paragraphs and no action blocks derives
`slack` from the *entire* narration — there is no action to subtract — so every
transition runs to `max_ms` and consumes real speech. With defaults that is a
600 ms crossfade against a 300 ms quiet window, overlapping 300 ms of the
outgoing sentence with the incoming one. The cap is what makes §3's claim true
that such a script's narration follows one segment after another.

The cap deliberately overrides `min_ms`: a transition shorter than the
configured minimum, or none at all, is better than one that talks over the
narration. A **fixed** duration is honoured as written, because the author
asked for it by name — but exceeding the quiet window emits a warning naming
the beat and the overlap, since the result is two voices at once.

### 6.4 Padding and directives

`lead_in` and `tail` (default 150 ms each) pad every narration clip so speech
does not butt against a transition. `<!-- teleprompt: pause 800ms -->` inserts
an explicit silent beat. Both participate in slack.

### 6.5 The Timeline

```jsonc
{
  "version": 1,
  "script": "script.md",
  "locale": "nl",
  "duration_ms": 74210,
  "generated_by": "teleprompt 0.4.1",
  "entries": [
    {
      "beat": "quickstart-1",
      "start_ms": 0,
      "duration_ms": 5820,
      "policy": "hold",
      "narration": {
        "segment": "welcome",
        "source_hash": "8f2a1c",
        "audio_hash": "c9e4...",
        "duration_ms": 5520,
        "voice_source": "recorded",
        "voice_source_actual": "cloned",
        "downgrade_reason": "take stale"
      },
      "action": {
        "span": "quickstart-1-a",
        "scene": "browser",
        "adapter": "playwright@0.3",
        "start_ms": 5670,
        "duration_ms": 150,
        "span_hash": "77bd...",
        "duration_source": "measured"
      },
      "transition": { "kind": "crossfade", "duration_ms": 300 }
    }
  ]
}
```

Committed to `timelines/<script>.<locale>.json`. This file is the review
surface: it is what a reviewer reads to understand the effect of a prose change,
and what CI compares against.

### 6.6 `diff`

`teleprompt diff` compares the computed timeline against the committed one (or
any git revision) and renders it as prose:

```
script.md (nl) — 72.4s → 74.2s (+1.8s)

  changed
    welcome        narration 4.2s → 5.8s   (text edited)
    ~ 6 beats after this point shift by +1.6s

  stale takes
    done           recorded 2026-08-02, text edited since
                   build will fall back to: cloned

  re-render
    3 of 24 beats need capture   (~40s estimated)
```

Structured output via `--format json` for CI.

## 7. Scenes

teleprompt does not define a language for describing visual steps. Each domain
already has one that is better than anything designed here would be: Playwright
for browsers, VHS for terminals, AppleScript for macOS UI. teleprompt defines a
**contract**, and each scene is an **adapter** satisfying it over an existing
tool.

This is a deliberate reversal of an earlier draft, which specified a single
teleprompt-owned step grammar. That design was rejected for three reasons: the
grammar would have been three domain sub-grammars sharing a tokenizer, so the
"one language" claim was illusory; matching Playwright's selector engine and
auto-waiting is a large project whose output would be worse; and `playwright
codegen` already implements most of the record-first flow in §8.

### 7.1 Scenes and adapters

Two layers, and the distinction matters because it is what keeps the artifact
portable:

- A **scene** names a domain — `browser`, `terminal`, `media`. This is what an
  author writes in the fence, and what appears in the timeline.
- An **adapter** implements a scene over a specific tool — `playwright`, `vhs`.
  Which adapter serves a scene is configuration, not part of the script.

```yaml
scene:
  browser:
    adapter: playwright        # the default
    base_url: "http://localhost:3000"
```

Only one adapter per scene ships initially, so the indirection buys nothing
today. It is there because the alternative — writing `scene=playwright` in every
fence — makes the choice of tool part of every script, and swapping it a
find-and-replace across the corpus. The block body is adapter-native regardless;
what the indirection protects is everything *around* the body.

### 7.2 The contract

Everything the compiler needs from a scene, and nothing more:

> **As shipped in M0.** The trait is split at the async boundary. M0 ships the
> compile-time half as `SceneCompiler: Send + Sync` — `kind`, `validate`,
> `spans`, `estimate`, all synchronous and IO-free, which is what `check` and
> `plan` need and all they need. `prepare`, `execute`, and `teardown` arrive in
> M1 alongside rendering, as a separate async trait; M0 deliberately renders no
> video, so an async runtime would have had no work to do. Two further
> departures from the sketch below: `spans` takes the block id as a second
> argument, because span ids are `{block_id}#{index}`; and both fallible
> methods return `Vec<Diagnostic>` rather than a single `ParseError`, so one
> `check` reports every bad line in a block instead of only the first.
>
> `BlockSource` also carries a `BodyOrigin` rather than a bare span: an
> `include=`d body's lines belong to the included file and are numbered from 1
> there, so diagnostics name that file rather than applying the fence's offset
> to a line that does not exist in the script.

```rust
#[async_trait]
pub trait Scene: Send {
    fn kind(&self) -> &'static str;

    /// Static validation of block source. Runs during `check`, before any
    /// process starts or credit is spent. Adapters delegate to whatever their
    /// tool provides — a tape parser, `tsc`, a syntax check.
    fn validate(&self, src: &BlockSource) -> Result<Validated, ParseError>;

    /// Split a block at its marks into spans. One span per beat.
    fn spans(&self, src: &Validated) -> Result<Vec<Span>, ParseError>;

    /// Duration a span needs at natural pace. Adapters that can compute this
    /// statically do; those that cannot return `Measured::Unknown` and the
    /// compiler measures once, then caches.
    fn estimate(&self, span: &Span) -> Measured;

    async fn prepare(&mut self, cfg: &SceneConfig) -> Result<()>;

    /// Execute one span into its scheduled slot. The scene may pace itself
    /// against `slot`; frames go to `rec`.
    async fn execute(&mut self, span: &Span, slot: Slot, rec: &mut Recorder)
        -> Result<Captured>;

    async fn teardown(&mut self) -> Result<()>;
}
```

Four obligations, and the design rests on all four:

**Marks** are how narration interleaves. A mark is a yield point where the
adapter hands control back to the scheduler, which holds until the narration
slot is satisfied. Without marks, a block is one opaque lump and none of the
scheduling policies in §6.2 can apply to it. Every adapter must express them.

**Duration** may be computed or measured. Adapters over declarative languages
compute it statically. Adapters over imperative ones return `Unknown`; the
compiler then runs a measuring pass on first build and caches per-span timings
keyed on the span's content hash. Measured numbers are truer than estimates —
the cost is a slower cold build, not a worse result.

**Identity** is a content hash per span, so re-capture is selective. Marks buy
granularity here too: editing one span re-captures one beat rather than the
whole block.

**Validation** delegates. Whatever the underlying tool already does — `tsc`, a
tape parser — is what `check` reports, mapped into teleprompt's diagnostic
format with the fence's line offset applied so errors point at the Markdown.

### 7.3 `media`

Images, clips, colour cards, title cards. The one scene with no existing tool
worth adopting, so it has a handful of directives. Durations are explicit, so
`estimate` is exact.

```
image src=diagrams/arch.png fit=contain
clip  src=b-roll/office.mp4 from=00:12 to=00:19
title text="Part Two" subtitle="Configuration"
mark
```

Ships first: it makes the whole pipeline demonstrable end to end before any
external runtime is involved.

### 7.4 `browser` — Playwright adapter

The block is a real Playwright script body. teleprompt runs a Node sidecar and
speaks to it over stdio JSON-RPC; `page` is a standard Playwright `Page`, and
`tp` is a small teleprompt helper.

```ts
await page.goto('/projects');
await tp.mark();                                  // narration plays, visual holds
await page.getByRole('button', { name: 'New' }).click();
await tp.mark('concurrent');
await tp.type('input[name=title]', 'Demo');       // paced to fill its slot
await tp.highlight('.cta');
await page.getByText('Ready').waitFor();
```

`tp.mark()` blocks until the scheduler releases it. Pacing-aware helpers
(`tp.type`, `tp.scroll`, `tp.zoom`, `tp.highlight`) read the current slot to
compute their own speed, which is how `stretch` reaches inside a script. Plain
Playwright calls run at natural pace and are simply measured.

`spans` splits on `tp.mark()` calls, found by parsing the block with a JS parser
rather than by regex, so marks inside comments or strings do not confuse it.
`validate` typechecks with `tsc` against teleprompt's ambient declarations for
`tp`. `estimate` returns `Unknown`.

Capture uses Playwright's own video recording per span rather than raw CDP
screencast, which removes the frame-timing risk the earlier draft carried.

Determinism remains a design obligation: fixed viewport and device scale factor,
animations suppressed via `prefers-reduced-motion` plus an injected CSS override,
fonts required locally, and `check` warning on bare `waitForTimeout` calls over
1 s as a pacing smell.

### 7.5 `terminal` — VHS adapter

The block is VHS tape syntax. teleprompt parses the tape and executes it against
its own PTY rather than shelling out to `vhs`, because VHS renders a whole tape
to one file and cannot resume — there would be no way to interleave narration.
The language is reused; the runtime is not.

> **As shipped.** Two departures from the sketch below, both found while
> writing the adapter. The mark is spelled `# mark`, a VHS comment, rather
> than a `Mark` command; and `estimate` returns `Estimated`, not `Exact`.
> Both are argued at the end of this section.

```
Set TypingSpeed 50ms
Type "cargo build --release"
Enter
Sleep 2s
# mark
Type "./target/release/acme --help"
Enter
```

**The mark is a comment, so the tape stays a tape.** An earlier draft added
`Mark` to the grammar and claimed that tapes without it remain valid VHS —
true, and beside the point, since every tape that *uses* the feature stops
being valid VHS. `include=` is what makes the distinction matter: the whole
reason to point a fence at a real `demo.tape` is that `vhs demo.tape` runs it,
editors highlight it, and VHS's own tooling understands it. A grammar addition
forfeits all three the moment an author reaches for a mark. `# mark` is inert
to VHS and costs nothing but the convention that this one comment is
reserved. Consistent with §7.1: the block body is adapter-native, and
teleprompt's additions live around it, not inside it.

**`estimate` returns `Estimated`, not `Exact`.** `Sleep` and `Set TypingSpeed`
are exact, and if they were all a tape contained the earlier claim would
hold. `Wait` is the exception: it blocks until the shell prompt returns, so
its duration is whatever the command underneath takes — `cargo build` in the
example above — and that number is nowhere in the tape. A tape without `Wait`
does estimate exactly; the adapter cannot promise that in general, and
`Estimated` is the honest signal to the compiler that M1's measuring pass has
something to improve. `Exact` is reserved for adapters whose language states
the whole truth about its own timing.

`Set TypingSpeed` remains the natural knob for `stretch-action`. Output
directives (`Output`, `Set Shell`) that conflict with teleprompt's own capture
are rejected by `validate` with an explanatory error.

### 7.6 Later adapters

**`macos`** — AppleScript or JXA. Imperative, unmeasurable, and genuinely flaky;
it satisfies the contract as an opaque span with a declared duration and
explicit marks. Not a v1 commitment.

**`asciinema`** — cast files are a recorded format rather than an authored one,
which makes them a natural import source (§8) more than a scene.

Adding an adapter touches no other crate. That is the property the contract
exists to protect.

## 8. Record-first authoring

The compilation pipeline assumes a script exists. Most scripts will not be
written from a blank file. The expected origin is that the author *does the
thing* while narrating, in one take, in flow — and then refines.

teleprompt supports this by observing rather than by importing an opaque
recording. Pixels cannot be re-driven; a structured trace can.

### 8.1 Capture

```
teleprompt import --scene browser --url http://localhost:3000
```

Opens a real browser, records the microphone, and captures a **trace** of actual
actions — clicks, navigations, typed input, scrolls — with timestamps. This is
`playwright codegen` with an audio track, and it is deliberately built on
codegen rather than reimplemented: selector generation is the hard part and
Playwright already does it well. The terminal equivalent wraps a PTY and records
a cast.

The trace is written to `.teleprompt/traces/` and is an intermediate, not a
committed artifact.

### 8.2 Derivation

From the trace plus the audio track:

1. **Transcribe** with word timings (Whisper-class ASR, local by default).
2. **Segment** at sentence boundaries, preferring natural pauses. Long pauses
   become segment breaks; very short ones do not.
3. **Interleave** actions and segments by timestamp into beats.
4. **Infer policy** from observed timing — an action inside speech becomes
   `concurrent`, one in a gap becomes `hold`, typing spread across a sentence
   becomes `stretch`. This is arithmetic on timestamps, not inference in the
   machine-learning sense.
5. **Emit** the Markdown script: prose paragraphs with promoted explicit IDs
   (§3.3), action blocks in the scene's own language with `tp.mark()` inserted
   at the derived beat boundaries, and inferred policies on the fences.
6. **Register takes** — the single continuous recording, sliced per segment,
   written to `takes/index.toml` with `source_hash` bindings.

Step 6 is why the take model must be a slice (§4.2). One recording serves the
whole script, and no audio is copied or re-encoded.

### 8.3 Why this composes

The emitted script has its `recorded` tier already populated, so the very first
`build` produces the author's real voice over their real demo. Refinement then
runs through machinery that already exists:

- Edit a paragraph → its take goes stale → the fallback ladder (§4.1) drops that
  segment to `cloned`, still the author's voice, now saying the better sentence.
  Untouched segments keep the genuine recording.
- The enrollment corpus for that clone is the take just captured (§4.4).
- `diff` reports what the edit did to the pacing before anything re-renders.
- Re-capture re-drives the app from the trace-derived script. The original screen
  recording is a reference and can be discarded.

None of this was designed for import; the pieces compose because the artifact,
not the recording, is the thing under version control.

### 8.4 Limits

**Import produces a first draft, not a shippable artifact.** Real takes contain
filler, backtracking, mouse wandering, and dead air. The output is meant to be
edited — by the author, or by an assistant working on the Markdown — and the
tooling makes no claim otherwise.

**Only replayable scenes can be imported.** Browser and terminal work because
their actions can be re-driven. Arbitrary desktop capture cannot, and is out of
scope.

**Segmentation is a heuristic** and will sometimes split awkwardly. Since the
output is Markdown, fixing it is editing a paragraph break — cheap, and the take
slices re-derive from the corrected boundaries.

## 9. Rendering

ffmpeg, orchestrated from Rust. teleprompt builds the filter graph and invokes
`ffmpeg` as a subprocess.

- Video: each captured beat becomes a segment; segments are concatenated with
  the scheduled transitions (`xfade` for crossfades, plain concat for cuts).
- Audio: narration clips placed at their timeline offsets over a silent bed
  (`adelay` + `amix`), with optional background music at a configured level.
- Subtitles: emitted as SRT and VTT from segment timings, per locale, for free.
  Word timings, where the backend provides them, produce word-level VTT cues.
- Output: H.264/AAC MP4 by default, configurable.

ffmpeg is invoked with an explicit argument vector, never a shell string. Its
availability and version are checked by `check` and `doctor`, since a missing or
too-old ffmpeg is the most likely first-run failure.

An optional HTML overlay pass — titles, captions, callouts rendered in a
headless browser and composited on top — is a natural later extension and is
deliberately excluded from v1.

## 10. CLI

Rust, `clap` derive. The binary is a thin shell over `teleprompt-core`; every
command is a function call into the library, and every command supports
`--format json` with a stable schema.

```
teleprompt new <name>              scaffold a project
teleprompt check [script]          parse and validate; no side effects, no cost
teleprompt doctor                  verify ffmpeg, browser, backends, credentials

teleprompt plan [script]           compile the timeline; print it
  --locale <tag>  --chapter <slug>  --format json
teleprompt diff [script]           compare against committed timeline
  --against <git-rev>  --format json
  --exit-code            exit 3 when the timeline has drifted (for CI)
teleprompt build [script]          full compile to video
  --locale <tag> (repeatable, or `all`)
  --chapter <slug>       build one chapter
  --watch                rebuild on change
  --dry-run              plan and report the work, render nothing
  --strict-voice         downgrades are fatal
  --no-cache

teleprompt say <segment>           synthesize one segment and play it
teleprompt voices                  list backends, voices, capabilities
teleprompt voice enroll            build a cloned voice profile
teleprompt record                  prompter capture
  --stale                only segments whose takes have drifted
  --chapter <slug>  --segment <id>
teleprompt takes ls|pick|rm        manage recordings

teleprompt import                  record a live session, derive a script
  --scene browser|terminal
  --url <url>            starting point for the browser scene
  --out <path>           script to write
  --no-audio             capture the trace only, narrate later
teleprompt trace ls|show|replay    inspect captured traces

teleprompt loc sync                reconcile translation sidecars
teleprompt loc status              per-locale coverage and staleness
teleprompt cache ls|clean
teleprompt serve                   local preview (later milestone)
```

Two loops, by design. The inner loop is `plan`/`diff` — offline, sub-second with
the `null` backend, run constantly while writing. The outer loop is `build
--watch` — real audio and real capture, run when the shape is settled.

### 10.1 Exit codes

`0` success · `1` runtime failure · `2` validation error (`check` failed) · `3`
drift (`diff` found changes under `--exit-code`, for CI) · `4` voice downgrade
under `--strict-voice`.

## 11. Repository layout

Committed:

```
teleprompt.toml
scripts/quickstart.md
scripts/quickstart.nl.md
scripts/quickstart.spec.ts     # optional: an `include`d Playwright script
timelines/quickstart.en.json
timelines/quickstart.nl.json
takes/index.toml               # manifest: segment → file, offset, hash
voices/jan.toml
```

Ignored (`.gitignore` written by `new`):

```
.teleprompt/cache/
.teleprompt/traces/
build/
takes/*.wav       # large; opt in to git-lfs per project
```

Script and timeline are committed; media is not. A reviewer can see from the
diff alone what a prose change did to the pacing, without cloning binaries. Take
*audio* is excluded by default because it is large, with a documented git-lfs
path for teams that want reproducible human narration — but `takes/index.toml`
is always committed, so a checkout without the audio still knows which segments
were recorded, by whom, and whether they have gone stale.

## 12. Crate structure

A workspace. The split follows the purity boundary: everything that can be a
pure function is, and lives where it can be tested without IO.

Shipped in M0 — six crates:

| crate | responsibility |
|---|---|
| `teleprompt-core` | AST, parser, segment identity, config resolution, `VoiceSource`, error types |
| `teleprompt-schedule` | policies, `Timeline`, diffing. Pure. |
| `teleprompt-voice` | `VoiceBackend` trait, capabilities, resolution ladder, `null` |
| `teleprompt-scene` | `SceneCompiler` contract, registry, `mock` adapter |
| `teleprompt-compile` | the seam: walks a `Program`, drives scene and voice, emits beats |
| `teleprompt-cli` | clap, output formatting, the `teleprompt` binary |

M1 and later:

| crate | responsibility |
|---|---|
| `teleprompt-voice-kokoro` | local TTS backend |
| `teleprompt-voice-elevenlabs` | API backend, cloning, enrollment |
| `teleprompt-scene-media` | images, clips, title cards. Pure Rust. |
| `teleprompt-scene-playwright` | Node sidecar, JSON-RPC protocol, mark splitting, `tp` helper |
| `teleprompt-scene-vhs` | tape parser, PTY execution, terminal frame rendering |
| `teleprompt-import` | trace capture, ASR, segmentation, policy inference, script emission |
| `teleprompt-render` | ffmpeg graph construction, muxing, subtitles |
| `teleprompt-cache` | content-addressed store, `Slot`/`Clock`/`Recorder` measurement cache |

`teleprompt-compile` exists because `core`, `scene`, `voice`, and `schedule`
must not depend on one another: without it, `schedule` would have to know about
`scene`, or `core` about both. Anything that needs two of them belongs in the
seam. `VoiceSource` lives in `core` rather than `voice` for the same reason —
`schedule` names it on every narration input, and shared vocabulary belongs
next to `Hash`, `Config`, and `SourceSpan`.

Plus one non-Rust package: `npm/teleprompt-driver`, the Node sidecar and the
`tp` helper library that browser action blocks import. It is the only JavaScript
in the project, and its protocol with `teleprompt-scene-playwright` is versioned
so the two can be upgraded independently.

Adapter crates are named for the tool they wrap, not the domain they serve —
`scene-playwright`, not `scene-browser`. A second browser adapter (or a
replacement) is then an additive change rather than a rename.

`core` and `schedule` have no async runtime, no network, and no filesystem
dependency beyond reading the script. The most intricate logic in the project is
consequently testable as plain functions over plain data. As shipped, M0 is
stricter still: `core` does not read the script either — the CLI does, and hands
`core` a `&str`.

Adapters and backends are Cargo features, default-on for `media`, `null`, and
`kokoro` only. The `playwright` feature is opt-in because it pulls a Node
runtime requirement; a user who only needs `media` and `null` builds a small
binary with no external runtime at all, which is exactly what M0 targets.

## 13. Interfaces

The CLI is v1. Every other interface is a client of the same library, and the
constraint that makes them cheap is that the CLI holds no logic of its own.

**Local web preview (`serve`)** — the recommended second interface, because it
is where the feedback loop wants to live. A timeline scrubber, per-segment
re-synthesize and replay, locale variants side by side, and a live diff panel
against the committed timeline. Delivered as a small HTTP server over the same
core, with the browser holding no state that is not derivable from the script.

**Desktop client (Tauri)** — the right home for the prompter and microphone.
Native audio input, a full-screen scrolling prompter with pace guidance,
waveform take review, and one-click re-record. Reuses the web preview's UI over
the same core, so it is an increment on `serve` rather than a parallel product.

**Editor extension (VS Code)** — inline "speak this paragraph", timeline
durations in the gutter, stale-take and stale-translation squiggles, jump from a
segment to its position in the rendered video. Speaks to the CLI's JSON mode; no
Rust linkage.

**CI** — `check` on every push; `diff --exit-code` to fail a PR whose committed
timeline is out of date; `build --strict-voice` on merge to publish. The
committed timeline is what makes this possible: CI can verify pacing without
rendering.

**Library** — `teleprompt-core` as a published crate, for embedding the compiler
in other tooling.

**Narration manifest (`dub`)** — the interface for pipelines that render
themselves. `teleprompt dub` writes narration audio and a versioned JSON
manifest describing what is said, when, and by which voice tier, so a tool
teleprompt does not control — Remotion, After Effects, a web player — can
consume the dubbing while owning its own picture. The drift gate still applies,
so prose edits fail CI there as they do here. Specified in
`2026-08-15-teleprompt-narration-manifest-design.md`.

## 14. Testing

- **Scheduler**: the bulk of the test suite. Table-driven cases over synthetic
  durations covering every policy, alignment, bound, and clamp. Pure functions,
  no fixtures, instant.
- **Parser**: golden files. A corpus of scripts, each with its expected AST and
  its expected diagnostics. Malformed input is as important as valid input.
- **`null` backend end-to-end**: full `parse → plan → diff` over a realistic
  multi-chapter, multi-locale script, entirely offline and deterministic. Runs
  in CI on every push.
- **Render**: ffmpeg graph construction is unit-tested by asserting on the
  generated argument vector rather than by rendering. A small number of real
  renders verify duration and stream layout via `ffprobe`, gated behind a
  feature so the default test run needs no ffmpeg.
- **Mark splitting**: golden tests per adapter. For Playwright, marks inside
  comments and string literals must *not* split — the case a regex would get
  wrong, and the reason splitting uses a real JS parser.
- **Adapters**: integration-tested against fixture pages and scripted shells,
  gated behind their Cargo feature and excluded from the default run. A `mock`
  adapter implementing the contract with scripted durations covers the
  compiler's side of the contract with no external runtime.
- **Cache**: property tests asserting the invariant that deleting any cache
  entry changes cost but never output.
- **Import**: golden tests over recorded trace fixtures plus canned transcripts,
  asserting the derived script — segmentation, inferred policies, take slices.
  Deterministic, since the trace and transcript are inputs rather than captured.

The `null` backend and the `mock` adapter are what make this tractable: the
majority of the system is verifiable with no network, no browser, no Node, no
ffmpeg, and no audio device.

## 15. Error handling

Errors are typed per crate (`thiserror`) and surfaced with source spans.
A validation failure names the file, line, and column, quotes the offending
line, and states what was expected — the standard a compiler is held to,
because that is what this is.

Failures are classified as **user error** (bad script, missing credential,
absent ffmpeg — actionable, no backtrace, exit 2), **environment failure**
(network, API error, browser crash — retried where safe, then reported with
context), or **internal error** (a bug; full backtrace and an issue link).

Long operations report progress per beat. An interrupted build leaves the cache
consistent: entries are written to a temporary path and atomically renamed, so a
partial write is never observed as a hit.

## 16. Milestones

Each produces something usable on its own.

**M0 — the loop, without video.** Parser, segment identity, config resolution,
`null` backend, the `Scene` contract with the `mock` adapter, scheduler with all
four policies, `Timeline`, and the commands `new`, `check`, `doctor`, `plan`,
`diff`. No rendering at all, and no external runtime — pure Rust.

This deliberately ships no video, and that ordering is the most important
decision in the plan. The timeline-and-diff machinery *is* the product; a
renderer built on top of a loop that does not yet feel right only reaches a bad
answer faster. At the end of M0, the core question — does editing prose produce
a legible, useful diff of the video's pacing? — is answerable.

**M1 — first real video.** `media` adapter, ffmpeg composition, `kokoro`, cache,
subtitles, `build`, `--watch`. A narrated video from a committed script. Also
`dub` and the narration manifest, which need a real voice backend and nothing
else — see `2026-08-15-teleprompt-narration-manifest-design.md`.

**M2 — Playwright adapter.** Node sidecar and protocol, the `tp` helper, mark
splitting, measurement pass, Playwright video capture, determinism controls. The
Playwright analogy becomes literal.

**M3 — voice spectrum and locales.** ElevenLabs, cloning, `voice enroll`, the
fallback ladder, `--strict-voice`, translation sidecars, `loc sync|status`,
per-locale builds.

**M4 — recording.** The prompter, the take manifest and slice model, hash
binding, staleness reporting, `record --stale`, take management. The `recorded`
tier lands, and the spectrum is complete.

**M5 — record-first.** `import`: trace capture over Playwright codegen, ASR,
segmentation, policy inference, script emission, take slicing. Depends on M2 for
the trace and M4 for the manifest, which is why it lands here despite being the
authoring path most creators will reach for first.

**M6 — breadth.** VHS adapter, `serve`, HTML overlay pass.

## 17. Risks

**Node runtime dependency.** The Playwright adapter needs Node and a Playwright
install, which is a real cost for a Rust CLI. Mitigated by making it an opt-in
Cargo feature: M0 and M1 need no external runtime, `doctor` diagnoses a missing
or mismatched install precisely, and the contract means a future pure-Rust
browser adapter is additive rather than a rewrite.

**Coupling to Playwright's API surface.** Adopting a tool means inheriting its
breaking changes, and the `tp` helper wraps internals that may shift. Mitigated
by pinning a supported Playwright range, versioning the sidecar protocol
independently of the crate, and keeping `tp` a thin layer over public API only.

**Measurement staleness.** Cached span measurements can drift from reality when
the application under test changes without the script changing — the app gets
slower, and the timeline silently no longer matches. Mitigated by recording
measured-versus-actual per capture and warning when they diverge beyond a
threshold, plus `build --remeasure` to discard the measurement cache.

**Kokoro operational surface.** A local model runtime is a real dependency with
real failure modes. Mitigated by running it as a subprocess against a local
server rather than in-process, so failures are isolated and diagnosable, and by
`doctor` checking it explicitly.

**Derived segment ID drift.** Positional IDs shift when segments are inserted,
which would silently invalidate takes. Mitigated by the warning in §3.3 and by
`loc sync --promote`. If drift proves common in practice, promotion becomes the
default at `record` time.

**Cross-lingual timing divergence.** A translated segment can be substantially
longer or shorter than its source, so a beat tuned for one locale may be poorly
paced in another. This is inherent to dubbing. Mitigated by scheduling each
locale independently — a locale is never forced to the source's timing — and by
`loc status` reporting per-locale duration deltas so an author can see which
translations need tightening.

**Import quality.** Derived scripts depend on ASR accuracy and on segmentation
heuristics, and a poor first draft could cost more to fix than writing from
scratch. Mitigated by treating the output as explicitly a draft (§8.4), by
keeping segmentation a paragraph break the author can move, and by the take
slices re-deriving from corrected boundaries rather than needing a re-record.

**Scope.** The adapter and backend extension points invite indefinite expansion.
Mitigated by the milestone ordering: the contract exists from M0, but only
`mock`, `media`, and `null` are required to prove the architecture.
