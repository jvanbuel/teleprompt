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
  --format wav|mp3       audio encoding (default wav)
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
| `segments[].start_ms`, `duration_ms` | Absolute placement on the narration timeline. `start_ms` already includes `lead_in` padding; `duration_ms` is the speech itself, excluding padding, so `start_ms + duration_ms` is exactly when the voice stops. |
| `segments[].audio` | Path relative to this manifest. |
| `voice_source` / `voice_source_actual` / `downgrade_reason` | The dubbing spectrum (§4) as delivered. `voice_source` is what the script asked for; `voice_source_actual` is what the ladder produced. `downgrade_reason` is `null` when they agree. A consumer can surface "this segment is machine-read" in a preview UI. |
| `source_hash` | Hash of the segment's prose. What `--check` compares. |
| `audio_hash` | Hash of the rendered audio bytes. Lets a consumer cache renders and skip re-encoding when only unrelated segments changed. |
| `segments[].words` | Optional, present only when the backend advertises `word_timings` (§4.3). Omitted entirely otherwise — never present as an empty array, so its absence is unambiguous. |

### 5.2 Determinism

The manifest is byte-stable for the same script, config, and backend: field
order is fixed by the struct, no timestamps are emitted, and floats do not
appear. `generated_by` carries the teleprompt version and therefore changes
across releases, which is the same behaviour the `Timeline` already has and is
visible in review rather than silent.

### 5.3 `--check` and drift

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
    {manifest.segments.map((seg, i) => {
      const from = msToFrames(seg.start_ms, 30);
      const next = manifest.segments[i + 1];
      const until = next
        ? msToFrames(next.start_ms, 30)
        : msToFrames(manifest.duration_ms, 30);
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

A segment's length is the difference between two rounded absolute starts, as
above — never the rounding of a duration. Rounding durations independently lets
error accumulate across a long video and drifts audio out of sync with picture
by the end. This is the single most likely mistake a consumer will make, which
is why the example computes `until - from` rather than rounding
`seg.duration_ms`.

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

- `teleprompt-schedule` gains `manifest.rs`: the `NarrationManifest` types and
  a projection from `Timeline`. It lives beside `timeline.rs` because it is
  derived from it and must not be able to drift from it.
- `teleprompt-cli` gains `cmd/dub.rs`.

Testing follows the M0 posture:

- Projection from a fixture `Timeline` to a manifest is pure and hermetic, and
  is snapshot-tested for byte stability.
- The `null` backend (§4.3) gives a full `dub` run with no network, no model,
  and no audio — enough to test layout, paths, drift detection, and exit codes
  offline.
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
