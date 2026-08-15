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

The Playwright analogy is deliberate: a script is a program, execution is
reproducible, and the artifact under version control is the script rather than
the recording.

### Goals

- A video is a deterministic function of a committed script plus a cache.
- Narration duration is the primary input to visual pacing.
- Narration ranges from stock TTS to a clone of the author's voice to the
  author reading live, mixable within one script.
- One script produces N locale variants, each with its own correct timing.
- Every generated artifact is content-addressed, so edits cost the minimum
  possible recomputation.

### Non-goals (v1)

- A timeline editor. teleprompt is a compiler; editing happens in the script.
- Motion-graphics authoring. Transitions are chosen, not designed.
- Hosting, publishing, or analytics.
- Real-time streaming. Output is a file.

## 2. Concepts

| Term | Meaning |
|---|---|
| **Script** | The Markdown source file. The unit of compilation. |
| **Chapter** | A Markdown heading and everything under it. Addressable build target. |
| **Segment** | One narration paragraph. Carries a stable ID. The atom of voice and translation. |
| **Action block** | A fenced ` ```teleprompt ` block. One or more visual steps for a scene. |
| **Beat** | A segment plus the action block that follows it. The scheduling unit. |
| **Program** | A script resolved for one locale: config merged, translations applied. |
| **Timeline** | Compiled schedule — absolute times for every clip and step. Committed. |
| **Take** | One recording of a human reading one segment. |
| **Voice source** | Which tier on the dubbing spectrum produced a segment's audio. |

## 3. Artifact format

### 3.1 Shape

Front matter configures. Prose paragraphs are segments. Fenced `teleprompt`
blocks are action blocks. Headings are chapters.

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

```teleprompt
goto /
wait_for text="Get started"
```

Click through here, and you land on the project wizard.

```teleprompt policy=concurrent align=start
click text="Get started"
wait_for url="/new"
```

Give it a name, and you are done. {#done voice.source=recorded}

```teleprompt policy=stretch
type into=input[name=title] text="My first project" cps=12
click text="Create"
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

Action block keys: `policy`, `align`, `scene`, `id`, `max_speedup` (bounds
`trim`), `max_stretch` and `min_stretch` (bound `stretch`).

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

- **`null`** — synthesizes silence and *estimates* duration from a
  words-per-minute model, punctuation-aware. No audio quality, but exact enough
  timing to iterate on pacing at zero cost. Also makes the entire scheduler test
  suite hermetic and offline. It is a first-class backend, not a test double.
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
against that target. It records the microphone, stores the result as
`takes/<segment-id>/<n>.wav` with a sidecar recording the `source_hash`, and
offers immediate re-record. Multiple takes are kept; the newest valid take wins
unless `takes/<segment-id>/pick` names one.

The take's *actual* duration is what enters the timeline. The loop is: plan with
estimates, record, re-plan with truth.

## 5. Compilation pipeline

```
parse ─► resolve ─► voice ─► schedule ─► capture ─► compose
```

1. **Parse** — Markdown to AST. Assign segment IDs, validate attributes, parse
   action steps against the target scene's grammar. Pure, no IO. This is where
   `check` stops.
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

### 5.1 Caching

One content-addressed store under `.teleprompt/cache/`, keyed by BLAKE3 of a
canonical description of the inputs:

| artifact | key inputs |
|---|---|
| audio clip | segment text, locale, backend ID, voice, speed, backend version |
| video segment | scene kind, resolved steps, viewport, scene config, slot duration |
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

| policy | semantics |
|---|---|
| `hold` (default) | Narration plays over the visual state left by the previous beat. The action runs after narration completes. Predictable, and correct for most explanatory content. |
| `concurrent` | Action and narration overlap. `align=start` (default) begins them together; `align=end` makes them finish together; `align=center` centres the shorter within the longer. Whichever ends first waits. |
| `stretch` | The action's internal delays scale by a uniform factor so the block's duration exactly equals the narration's. Intended for typing, scrolling, and progress animations. Bounded by `max_stretch` (default 3.0) and `min_stretch` (default 0.33); exceeding either is a warning and the bound is applied. |
| `trim` | Narration is authoritative. An over-long action is time-compressed in post, bounded by `max_speedup` (default 2.0). Beyond the bound, the tail is cut at the last completed step and a warning is emitted. |

### 6.3 Slack and automatic transitions

The slack of a beat is the difference between its narration duration and its
action duration. This is the mechanism by which text length informs the visuals.

With `transition.duration: auto`, the transition into the next beat is
`clamp(slack * 0.5, min_ms, max_ms)` — generous pauses get a visible crossfade,
tight ones get a hard cut. Authors who want fixed pacing set an explicit
duration; the automatic mode exists so that the default behaviour of editing
prose is that the video's rhythm follows the writing.

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
        "block": "quickstart-1-a",
        "scene": "browser",
        "start_ms": 5670,
        "duration_ms": 150,
        "steps_hash": "77bd..."
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

```rust
#[async_trait]
pub trait Scene: Send {
    fn kind(&self) -> &'static str;

    /// Validate and lower raw step syntax. Called during parse — before any
    /// process starts, network call is made, or credit is spent.
    fn parse_steps(&self, raw: &[RawStep]) -> Result<Vec<Step>, ParseError>;

    /// Duration this block needs at natural pace, for scheduling. May be an
    /// estimate; the scheduler records it as such.
    fn estimate(&self, steps: &[Step]) -> SceneEstimate;

    async fn prepare(&mut self, cfg: &SceneConfig) -> Result<()>;

    /// Execute against a slot. The Clock reports timeline position so the scene
    /// can pace itself; the Recorder receives frames.
    async fn execute(&mut self, steps: &[Step], slot: Slot, rec: &mut Recorder) -> Result<Captured>;

    async fn teardown(&mut self) -> Result<()>;
}
```

`parse_steps` running at parse time is the single most valuable property here.
An unknown step, a malformed selector, or a missing required argument is caught
by `check` in milliseconds, rather than after a browser has launched and a
minute of TTS has been billed.

### 7.1 `media`

Static images, video clips, colour cards, title cards. No automation. Ships
first because it makes the whole pipeline demonstrable — a real narrated video
end to end — before any browser is involved.

```
image src=diagrams/arch.png fit=contain
clip src=b-roll/office.mp4 from=00:12 to=00:19
title text="Part Two" subtitle="Configuration"
```

### 7.2 `browser`

Chrome over CDP (`chromiumoxide`). Frames captured via `Page.startScreencast`,
each carrying a CDP timestamp; the recorder writes a frame list with
presentation times and ffmpeg resamples to the output frame rate.

```
goto /projects
click text="New project"
click selector="#submit"
type into="input[name=title]" text="Demo" cps=12
hover selector=".card:first-child"
scroll to="#pricing" duration=1200ms
wait_for text="Ready"
wait_for url="/new"
wait_for selector=".spinner" state=hidden
wait 500ms
highlight selector=".cta" style=box
zoom to=".card" scale=1.6 duration=600ms
eval "window.__demo.seed()"
```

Determinism is a design obligation, not a hope. Fixed viewport and device scale
factor; animations disabled via `prefers-reduced-motion` and a CSS override
injected at document start; a fixed clock and seeded randomness injected into
the page where the page cooperates; fonts required to be locally available.
`wait_for` is always preferred over `wait`, and `check` warns on bare `wait`
calls longer than 1 s as a pacing smell.

**Known limitation.** Screencast frame delivery is not perfectly uniform under
load. Capture is therefore treated as approximately-timed and resampled, with
the timeline — not the wall clock — as the authority on when each step begins. A
frame-accurate deterministic capture path (CDP `beginFrame` stepping) is
recorded as a future option in §12, not a v1 commitment.

### 7.3 `terminal`

A PTY running a shell, rendered to frames by a headless terminal emulator.
Typing is animated at a configurable characters-per-second so it reads as human.

```
run "cargo build --release" cps=18
expect "Finished"
send_keys "C-c"
clear
```

Later milestone; the browser scene proves the harder capture problem first.

## 8. Rendering

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

## 9. CLI

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

teleprompt loc sync                reconcile translation sidecars
teleprompt loc status              per-locale coverage and staleness
teleprompt cache ls|clean
teleprompt serve                   local preview (later milestone)
```

Two loops, by design. The inner loop is `plan`/`diff` — offline, sub-second with
the `null` backend, run constantly while writing. The outer loop is `build
--watch` — real audio and real capture, run when the shape is settled.

### 9.1 Exit codes

`0` success · `1` runtime failure · `2` validation error (`check` failed) · `3`
drift (`diff` found changes under `--exit-code`, for CI) · `4` voice downgrade
under `--strict-voice`.

## 10. Repository layout

Committed:

```
teleprompt.toml
scripts/quickstart.md
scripts/quickstart.nl.md
timelines/quickstart.en.json
timelines/quickstart.nl.json
voices/jan.toml
```

Ignored (`.gitignore` written by `new`):

```
.teleprompt/cache/
build/
takes/            # large; opt in to git-lfs per project
```

Script and timeline are committed; media is not. A reviewer can see from the
diff alone what a prose change did to the pacing, without cloning binaries. Takes
are excluded by default because they are large, with a documented git-lfs path
for teams that want reproducible human narration.

## 11. Crate structure

A workspace. The split follows the purity boundary: everything that can be a
pure function is, and lives where it can be tested without IO.

| crate | responsibility |
|---|---|
| `teleprompt-core` | AST, parser, segment identity, config resolution, error types |
| `teleprompt-schedule` | policies, `Timeline`, diffing. Pure. |
| `teleprompt-voice` | `VoiceBackend` trait, capabilities, resolution ladder, `null` |
| `teleprompt-voice-kokoro` | local TTS backend |
| `teleprompt-voice-elevenlabs` | API backend, cloning, enrollment |
| `teleprompt-scene` | `Scene` trait, registry, `Slot`/`Clock`/`Recorder` |
| `teleprompt-scene-media` | images, clips, title cards |
| `teleprompt-scene-browser` | CDP driving and screencast capture |
| `teleprompt-scene-terminal` | PTY driving and frame rendering |
| `teleprompt-render` | ffmpeg graph construction, muxing, subtitles |
| `teleprompt-cache` | content-addressed store |
| `teleprompt-cli` | clap, output formatting, the `teleprompt` binary |

`core` and `schedule` have no async runtime, no network, and no filesystem
dependency beyond reading the script. The most intricate logic in the project is
consequently testable as plain functions over plain data.

Backends and scenes are Cargo features, default-on for `media`, `null`, and
`kokoro`. A user who only needs `null` and `media` can build a small binary with
no browser dependency.

## 12. Interfaces

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

## 13. Testing

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
- **Scenes**: browser and terminal scenes are integration-tested against fixture
  pages and scripted shells, gated behind a feature and excluded from the
  default run.
- **Cache**: property tests asserting the invariant that deleting any cache
  entry changes cost but never output.

The `null` backend is what makes this tractable: the majority of the system is
verifiable with no network, no browser, no ffmpeg, and no audio device.

## 14. Error handling

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

## 15. Milestones

Each produces something usable on its own.

**M0 — the loop, without video.** Parser, segment identity, config resolution,
`null` backend, scheduler with all four policies, `Timeline`, and the commands
`new`, `check`, `doctor`, `plan`, `diff`. No rendering at all.

This deliberately ships no video, and that ordering is the most important
decision in the plan. The timeline-and-diff machinery *is* the product; a
renderer built on top of a loop that does not yet feel right only reaches a bad
answer faster. At the end of M0, the core question — does editing prose produce
a legible, useful diff of the video's pacing? — is answerable.

**M1 — first real video.** `media` scene, ffmpeg composition, `kokoro`, cache,
subtitles, `build`, `--watch`. A narrated video from a committed script.

**M2 — browser.** CDP driving, screencast capture, determinism controls, the
full step vocabulary. The Playwright analogy becomes literal.

**M3 — voice spectrum and locales.** ElevenLabs, cloning, `voice enroll`, the
fallback ladder, `--strict-voice`, translation sidecars, `loc sync|status`,
per-locale builds.

**M4 — recording.** The prompter, take storage and hash binding, staleness
reporting, `record --stale`, take management. The `recorded` tier lands, and the
spectrum is complete.

**M5 — breadth.** Terminal scene, `serve`, HTML overlay pass.

## 16. Risks

**Browser capture determinism.** Screencast frame timing varies under load.
Mitigated by treating capture as approximately-timed with the timeline as
authority, and by aggressive animation suppression. If this proves inadequate in
M2, the fallback is CDP `beginFrame` stepping — frame-accurate but substantially
more complex and slower.

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

**Scope.** The scene and voice extension points invite indefinite expansion.
Mitigated by the milestone ordering: the extension points exist from M0, but only
`media` and `null` are required to prove the architecture.
