# teleprompt narration manifest — design

**Status:** accepted, unimplemented
**Depends on:** `docs/superpowers/specs/2026-08-15-teleprompt-design.md` (the
core design; section references below are to that document)
**Milestone:** M1

## 1. Summary

teleprompt gains one command and one published file format:

```bash
teleprompt dub script.md --out public/narration/
```

`dub` runs the compilation pipeline as far as audio and stops. It writes the
narration audio and a **narration manifest** — a small, versioned, documented
JSON file describing what is said, when, for how long, and by which voice
tier — and renders no video.

The manifest exists so that a tool teleprompt does not control can consume
teleprompt's dubbing. Remotion is the motivating consumer and the worked
example in §7, but nothing in the format is Remotion-specific: After Effects,
DaVinci Resolve, a web player, or a Blender pipeline can read the same file.

This is the whole integration. teleprompt ships no Remotion code, takes no
Remotion dependency, bundles no Remotion project, and runs no Node process on
its behalf.

### Goals

- Deliver teleprompt's distinctive machinery — the dubbing spectrum, the
  fallback ladder, hash-bound take staleness, per-locale variants, and the
  drift gate — to a video pipeline that renders itself.
- Keep the artifact-first loop intact across the boundary: a committed
  manifest that no longer matches its script fails CI exactly as a committed
  timeline does.
- Cost teleprompt nothing in dependencies, runtimes, or licensing.

### Non-goals

- Rendering video. `dub` produces audio and JSON.
- Any knowledge of the consumer's visuals, layout, or composition graph.
- An npm package. See §8.
- A narration-constrained scheduling mode (fitting speech into a fixed visual
  slot). See §9.

## 2. Why this shape

Three integrations were considered.

**Remotion as compositor**, replacing ffmpeg in §9. Rejected. Every teleprompt
user rendering any video — including a plain screen recording with narration —
would be operating Remotion, which makes a third-party commercial licence a
condition of using teleprompt at all. Section 6 covers the licence terms.

**Remotion as a scene adapter** (`scene=motion`), a per-beat visual source
alongside `browser` and `terminal`. Viable and still open, but it puts a Node
sidecar, a webpack bundle, bundle-hash invalidation, and a fixture Remotion
project inside this repository, in exchange for teleprompt orchestrating
Remotion beat by beat. Deferred; §9 records what it would have been.

**Remotion consuming teleprompt.** The user's Remotion project is the host;
teleprompt is a build step that runs before `remotion render`. The composition
derives its own duration from the manifest, so *narration still drives
pacing* — the thesis executes inside Remotion rather than inside teleprompt.

The third is strictly cheaper than the second, carries no licence surface, and
generalises past Remotion for free. It is also reversible: if beat-by-beat
orchestration turns out to be what people want, the manifest will have shown
what they need and the adapter is still there to build.

## 3. `teleprompt dub`

```
teleprompt dub [script]            synthesize narration; write audio + manifest
  --out <dir>            output root (required)
  --locale <tag>         repeatable, or `all`; defaults to the script's locale
  --chapter <slug>       one chapter only
  --format wav           audio encoding (only value in v1; see below)
  --check                do not write; exit 3 if the manifest on disk is stale
  --strict-voice         downgrades are fatal (exit 4)
```

`dub` is `build` truncated before composition. It shares the compile pipeline
of §5 exactly: parse, resolve config, resolve voice tier through the fallback
ladder of §4.1, synthesize, schedule. It then serialises the narration
projection of the resulting `Timeline`.

**Action blocks are scheduled, not ignored.** A script containing
`scene=browser` blocks produces a manifest whose narration timings include the
gaps those blocks occupy, because those are the timings a full `build` would
produce and a manifest that disagreed with `plan` would be a trap. A script
written for this workflow simply has no action blocks, and then narration
segments follow one another back to back, separated only by padding (§6.4) and
explicit pauses. Adapter availability is enforced as in `check`: if a script
names an adapter that is not installed, `dub` fails rather than guessing.

Exit codes follow §10.1 unchanged: `0` success, `1` runtime failure, `2`
validation error, `3` manifest drift under `--check`, `4` voice downgrade under
`--strict-voice`.

**WAV only in v1.** `wav` is 16-bit PCM, written by a small encoder in this
repository, which keeps the output byte-stable and adds no dependency. MP3 or
AAC would each pull in an encoder binding for a format every consumer can
already transcode to, so `--format` exists to make the field non-breaking
later rather than to offer a choice today.

**Audio is produced by a second call, not by `synthesize`.** The
`VoiceBackend` trait (§4.3) gains one method:

```rust
/// Render this request to interleaved 16-bit PCM. Backends that cannot
/// produce audio return `Ok(None)`.
fn render_pcm(&self, req: &SynthRequest) -> Result<Option<Pcm>, VoiceError>;

pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}
```

`synthesize` stays timing-only. The inner loop of §10 — `plan` and `diff`,
which must remain sub-second — calls only `synthesize` and never allocates an
audio buffer. Only `dub` and `build` call `render_pcm`.

The `null` backend returns silence of exactly its estimated duration, as §4.3
already describes it doing. That makes `dub` fully functional before any real
synthesizer exists: a consumer gets correctly-timed silent narration, which is
a usable preview and an exactly-testable artifact.

## 4. Output layout

```
<out>/
  en/
    narration.json
    audio/
      welcome.wav
      provenance.wav
  nl/
    narration.json
    audio/
      welcome.wav
      provenance.wav
```

One directory per locale, each self-contained. Audio paths inside a manifest
are relative to that manifest, so a consumer resolves them without knowing the
output root, and a whole locale directory can be moved or served from anywhere.

Segment file names are the segment id (§3.3), which is already unique within a
script and already stable across edits that do not rename it.

### 4.1 What to commit

The manifest is small and diffable and **should be committed**. The audio is
neither.

For `synthetic` and `cloned` tiers, audio is reproducible from the script and
the backend, so the recommended posture is to gitignore `audio/` and regenerate
it — in CI, in a pre-render step, or on checkout. This does mean a consumer's
CI needs a working voice backend to render.

For the `recorded` tier the audio is a human performance and is not
reproducible by anything. Those takes live in the take manifest (§4.2, §4.5)
and are committed there; `dub` copies rather than synthesizes them.

Committing the manifest while regenerating audio is what makes `--check`
meaningful: the manifest is the reviewable claim, the audio is its output.

## 5. The manifest

`narration.json`, one per script per locale.

```jsonc
{
  "manifest_version": 1,
  "script": "script.md",
  "locale": "en",
  "generated_by": "teleprompt 0.4.1",
  "duration_ms": 40050,
  "audio": {
    "format": "wav",
    "sample_rate": 48000,
    "channels": 1
  },
  "chapters": [
    { "id": "quickstart", "title": "Quick start", "start_ms": 0 },
    { "id": "provenance", "title": "Provenance",  "start_ms": 12300 }
  ],
  "segments": [
    {
      "id": "welcome",
      "text": "Every video in this repository is built from a script you can read.",
      "chapter": "quickstart",
      "start_ms": 0,
      "duration_ms": 12200,
      "audio": "audio/welcome.wav",
      "voice_source": "recorded",
      "voice_source_actual": "cloned",
      "downgrade_reason": "take stale",
      "source_hash": "8f2a1c…",
      "audio_hash": "c9e4…",
      "words": [
        { "text": "Every", "start_ms": 0,   "end_ms": 320 },
        { "text": "video", "start_ms": 320, "end_ms": 610 }
      ]
    }
  ]
}
```

Hashes are BLAKE3, serialised as 64 lowercase hex characters; they are
abbreviated above for readability, as in §6.5.

### 5.1 Field semantics

| field | meaning |
|---|---|
| `manifest_version` | Integer, incremented on breaking change. Independent of the `Timeline`'s `version` — they are separate contracts with separate audiences. A consumer must refuse a version it does not know. |
| `script`, `locale`, `generated_by` | Provenance, mirroring the `Timeline` header. |
| `duration_ms` | Total length of the narration timeline, including any gaps left by action blocks and pauses. A consumer that derives its own duration from this gets a video exactly as long as teleprompt scheduled. |
| `audio.format` / `sample_rate` / `channels` | Uniform across every segment in the manifest. Stated so a consumer need not probe the files. |
| `chapters` | Derived from headings (§3.1). Present for consumers building chapter markers or navigation; empty array when the script has no headings. |
| `segments[].id` | The segment id from §3.3. Stable across edits that do not rename it, and the join key for everything else. |
| `segments[].text` | The narration prose as parsed. This is what makes captions possible without re-parsing the script. |
| `segments[].chapter` | Slug of the chapter this segment was spoken in, matching a `chapters[].id` when that chapter has one. Always present. Published rather than left to be reconstructed from timestamps, because `chapters` omits silent chapters and that reconstruction is therefore lossy. |
| `segments[].start_ms`, `duration_ms` | Absolute placement on the narration timeline. `start_ms` already includes `lead_in` padding; `duration_ms` is the speech itself, excluding padding, so `start_ms + duration_ms` is exactly when the voice stops. **Authoritative for this segment's length — see §5.2.** |
| `segments[].audio` | Path relative to this manifest. |
| `voice_source` / `voice_source_actual` / `downgrade_reason` | The dubbing spectrum (§4) as delivered. `voice_source` is what the script asked for; `voice_source_actual` is what the ladder produced. `downgrade_reason` is `null` when they agree. A consumer can surface "this segment is machine-read" in a preview UI. |
| `source_hash` | Hash of the segment's prose. What `--check` compares. |
| `audio_hash` | Hash of the encoded audio file named by `segments[].audio` — the bytes on disk, nothing else. Lets a consumer cache renders and skip re-encoding when only unrelated segments changed. Deliberately *not* the voice backend's synthesis cache key: that key embeds the teleprompt version, so publishing it would change every segment's hash on every teleprompt release and report "audio changed" on every segment of every consumer's next pull request. Two segments may legitimately share a hash — that means their audio is byte-identical, which is precisely when a cached render is reusable. With the `null` backend (silence) every segment of equal duration shares one; real speech does not. |
| `segments[].words` | Optional, present only when the backend advertises `word_timings` (§4.3). Omitted entirely otherwise — never present as an empty array, so its absence is unambiguous. |

### 5.2 A segment's own `duration_ms` is authoritative

**Never infer a segment's length from the next segment's `start_ms`.** Place
each segment's audio at its `start_ms` and give it its own `duration_ms`.

For a script of plain paragraphs the two agree: core spec §6.3 caps an `auto`
transition at the quiet window, so `segments[i].start_ms + duration_ms` equals
`segments[i + 1].start_ms` exactly and the narration reads as continuous
speech.

They stop agreeing as soon as anything else is going on, and in both
directions:

- A **gap** opens wherever an action block, a `pause` directive, or a
  `concurrent` beat puts time between two spoken segments. Most real scripts
  have these.
- An **overlap** appears when an author sets a *fixed* transition wider than
  the quiet window. That is honoured as written — they asked for a crossfade of
  that length by name — and the scheduler warns that narration will overlap.

So `next.start_ms - seg.start_ms` is wrong in the common case (it stretches a
segment across a gap it should not fill) and wrong in the rare one (it clips a
deliberate overlap). An earlier draft of the §7 example computed lengths that
way; it was wrong, and this rule is the fix.

An earlier version of this section documented a third cause: uncapped `auto`
transitions made *every* narration-only script overlap by 300 ms. That was a
scheduler defect, found by the whole-branch review of this feature and fixed in
core spec §6.3. The guidance here did not change when the defect was fixed,
which is the point of stating the rule rather than the symptom.

### 5.3 Determinism

The manifest is byte-stable for the same script, config, and backend: field
order is fixed by the struct, no timestamps are emitted, and floats do not
appear. `generated_by` carries the teleprompt version and therefore changes
across releases, which is the same behaviour the `Timeline` already has and is
visible in review rather than silent.

### 5.4 `--check` and drift

`teleprompt dub --check` recomputes the manifest, compares it to the one on
disk, and exits `3` on any difference, printing a report in the shape of §6.6:

```
narration: 34.8s → 40.1s (+5.3s)

changed:
  deploy           4.9s → 10.8s  (text edited)

needs re-render:
  deploy
```

This is the mechanism that keeps teleprompt's central claim working across the
integration boundary. A pull request that edits prose without regenerating the
manifest fails, exactly as one that edits prose without regenerating the
timeline fails today.

## 6. Licensing posture

Remotion is free for individuals, for-profit organisations with up to three
employees, non-profits, and evaluation. Beyond that a company licence is
required, and the threshold counts personnel who operate the software,
aggregated across collaborating parties.

Under this design teleprompt has **no exposure of any kind**: it ships no
Remotion code, redistributes nothing, and never invokes `renderMedia()`. Users
who never use Remotion never encounter it. Users who do are in exactly the
licence position they were already in by choosing Remotion, and teleprompt
neither improves nor worsens it.

One clause bears on the roadmap regardless. Remotion's FAQ names as an
unacceptable use case *allowing users to submit any Remotion video to your
server for rendering*. Should `serve` (§13, M6) ever grow hosted rendering,
it must not render user-supplied Remotion projects. Local rendering is
unaffected. This constraint is recorded here because it is easy to violate
accidentally later, when the reason has been forgotten.

## 7. Worked example: Remotion

Documentation, not shipped code. Given `public/narration/en/narration.json`:

```tsx
import { Composition, staticFile } from 'remotion';

export const RemotionRoot = () => (
  <Composition
    id="Tour"
    component={Tour}
    fps={30}
    width={1920}
    height={1080}
    defaultProps={{ manifestPath: 'narration/en/narration.json' }}
    calculateMetadata={async ({ props }) => {
      const manifest = await (
        await fetch(staticFile(props.manifestPath))
      ).json();
      if (manifest.manifest_version !== 1) {
        throw new Error(`unsupported manifest version ${manifest.manifest_version}`);
      }
      return {
        durationInFrames: msToFrames(manifest.duration_ms, 30),
        props: { ...props, manifest },
      };
    }}
  />
);
```

```tsx
import { AbsoluteFill, Sequence, staticFile } from 'remotion';
import { Audio } from '@remotion/media';

const Tour = ({ manifest }) => (
  <AbsoluteFill>
    {manifest.segments.map((seg) => {
      // Each segment's own start and end, as absolute offsets. Not
      // `next.start_ms` — segments may overlap (§5.2), and deriving a
      // length from the next start clips the tail of the speech.
      const from = msToFrames(seg.start_ms, 30);
      const until = msToFrames(seg.start_ms + seg.duration_ms, 30);
      return (
        <Sequence key={seg.id} from={from} durationInFrames={until - from}>
          <Audio src={staticFile(`narration/en/${seg.audio}`)} />
          <Slide segment={seg.id} />
        </Sequence>
      );
    })}
  </AbsoluteFill>
);
```

### 7.1 The rounding rule consumers must follow

`msToFrames` is `Math.round(ms * fps / 1000)`, and it must be applied to
**absolute offsets only**:

```ts
const msToFrames = (ms: number, fps: number) => Math.round((ms * fps) / 1000);
```

A segment's length is the difference between two rounded absolute offsets, as
above — never the rounding of a duration. Rounding durations independently lets
error accumulate across a long video and drifts audio out of sync with picture
by the end. This is the single most likely mistake a consumer will make, which
is why the example computes `until - from` rather than rounding
`seg.duration_ms`.

The two offsets are this segment's **own** start and end —
`seg.start_ms` and `seg.start_ms + seg.duration_ms` — added in milliseconds
and rounded separately. They are not `seg.start_ms` and `next.start_ms`:
consecutive segments may overlap (§5.2), so the next segment's start is not
this one's end, and using it truncates the tail of every segment but the
last. Both rules apply at once, and neither substitutes for the other.

teleprompt cannot enforce this; the manifest is milliseconds and frame rate is
the consumer's business. It is documented here and in the `dub` command's help
text.

### 7.2 What the consumer owns

Everything visual, and the mix. Remotion renders its own audio too, so a
project may already place music or interface sounds; the narration `<Audio>`
tags simply join that mix. A consumer that wants teleprompt's narration as a
separate stem instead can render the composition muted and mux externally.

## 8. No npm package in v1

A helper package (`useNarration()`, a `<Narration>` component, a
`calculateNarrationMetadata()` shim) is an obvious convenience and is
deliberately excluded.

The manifest is plain JSON and the example above is the whole integration —
roughly ten lines of consumer code. A package would take `remotion` as a peer
dependency, which is the only place in this design where teleprompt would
acquire a Remotion relationship at all, and it would need versioning against
both teleprompt and Remotion.

Ship the schema, watch what consumers actually write, and revisit. If a helper
does land it belongs in its own repository, not this one.

## 9. Deferred

**`scene=motion`, a Remotion scene adapter.** Beat-by-beat orchestration, with
Remotion compositions as visual spans. The design sketched during brainstorming
had the fence naming a composition and driving its props across `mark`
boundaries; `Measured` would gain an `Elastic { natural_ms }` variant, since a
composition re-rendered at a new length is re-timed rather than resampled and
so is properly exempt from the `max_stretch` clamp of §6.2. Revisit only if
manifest consumers ask for it.

**A narration-constrained policy.** Every policy in §6.2 treats narration as
the fixed side. Dubbing a video whose duration is already fixed inverts that:
the visual imposes a budget and over-long prose is a diagnostic — *"deploy:
narration 10.8s exceeds its 6.0s slot by 4.8s"* — never a sped-up voice. This
would also allow one render to serve many locales, since the picture would stop
depending on narration length. It is out of scope here because a manifest
consumer derives its duration *from* the manifest and therefore never
constrains it. It becomes necessary the moment someone wants to dub a video
they cannot re-time, and it is a change to `teleprompt-schedule`, not an
adapter.

## 10. Crate structure and testing

No new crate.

- `teleprompt-compile` gains `manifest.rs`: the `NarrationManifest` types and
  the projection that builds one.

  An earlier draft put this in `teleprompt-schedule`, beside `timeline.rs`, on
  the reasoning that the manifest is derived from the `Timeline` and should not
  be able to drift from it. That reasoning was wrong about the inputs. A
  manifest is a **join**, not a projection: timings come from the `Timeline`,
  but `text`, `chapters`, and `words` are nowhere in it — narration text and
  chapter titles live in the `Program`, and word timings live in the
  `SynthResult` that the scheduler discards. `teleprompt-compile` is the one
  crate holding all three, and §12 of the core design already names it the seam
  where the others meet. Drift protection comes from the test in this section,
  not from file adjacency.

- `teleprompt-compile`'s `CompileOutput` gains `narration: Vec<NarrationDetail>`
  carrying per-segment `text`, `chapter` (slug and index), `synth_request`, and
  `word_timings`, all of which `compile` already has in hand and currently
  drops. The `SynthRequest` is carried rather than rebuilt by the caller so
  that the audio `dub` renders and the duration the manifest publishes come
  from one object; the chapter *index* is carried because slugs derive from
  titles and two identically-titled chapters share one.
- `teleprompt-core`'s `Program` gains chapter provenance: `Item::Narration`
  gains a `chapter` slug and `Program` gains an ordered `chapters` list.
  `resolve` flattens the chapter tree today and keeps nothing of it, so the
  manifest's `chapters` field is otherwise unreachable.
- `teleprompt-voice` gains `Pcm`, `VoiceBackend::render_pcm`, and a 16-bit PCM
  WAV encoder.
- `teleprompt-cli` gains `cmd/dub.rs`.

Testing follows the M0 posture:

- Building a manifest from a fixture `Timeline` plus fixture narration details
  is pure and hermetic, and is snapshot-tested for byte stability.
- A test asserts every manifest segment's `start_ms` and `duration_ms` equal
  the corresponding `Timeline` narration entry's, so the join cannot drift from
  the schedule it describes.
- The `null` backend (§4.3) gives a full `dub` run with no network and no
  model — silent WAVs of exactly the estimated durations — enough to test
  layout, paths, audio bytes, drift detection, and exit codes offline.
- The WAV encoder is tested against a hand-computed 44-byte header, so the
  output is verified as a file format rather than only as a round trip through
  its own reader.
- A round-trip test deserialises a committed manifest and asserts an unknown
  `manifest_version` is refused rather than partially parsed.
- The TypeScript in §7 is checked as documentation, not executed; teleprompt's
  test suite acquires no Node dependency.

## 11. Risks

**The manifest becomes a de facto API.** Once consumers exist, changing it
costs them work. Mitigated by `manifest_version` and by keeping the surface
small — the temptation will be to add visual or beat-level fields, and the
answer should usually be no, because the `Timeline` already exists for that
audience.

**Regenerated audio in consumer CI.** A consumer that gitignores `audio/` needs
a voice backend at render time. With `kokoro` that is a local model download in
their pipeline; with `elevenlabs` it is credentials and per-render cost. The
alternative — committing audio — is large binaries in git. Neither is wrong,
and §4.1 states the trade rather than choosing for them.

**Drift between `dub` and `build`.** Two commands that must schedule
identically. Mitigated by sharing one pipeline and by a test that asserts a
manifest's segment timings equal the corresponding `Timeline` narration
timings for the same script.
