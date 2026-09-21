# Rendering a teleprompt script with Remotion

teleprompt publishes narration and its timing; Remotion owns the picture. This
is the whole integration, and the seam between them is one file:
`narration.json`.

```bash
teleprompt dub scripts/tour.md --out public/narration
```

```
public/narration/en/narration.json
public/narration/en/audio/welcome.wav
```

`--out public/narration` rather than anywhere else because Remotion's
`staticFile()` serves from `public/`.

**The example below is not compiled by this repository's CI.** teleprompt takes
no Remotion dependency and the workspace ships no Node, which is deliberate
(README: "No network, no browser, no Node, and no ffmpeg are required"). The
code here is copy-paste material, kept short enough to audit by eye. If it
drifts from the manifest, the manifest is right and this document is wrong —
`crates/teleprompt-compile/src/manifest.rs` is the source of truth, and
`manifest_version` moves when its shape does.

## What the manifest gives you

```jsonc
{
  "manifest_version": 1,
  "script": "tour.md",
  "locale": "en",
  "generated_by": "teleprompt 0.1.0",
  "duration_ms": 322730,
  "audio": { "format": "wav", "sample_rate": 24000, "channels": 1 },
  "chapters": [
    { "id": "the-manual-is-the-program", "title": "The manual is the program", "start_ms": 150 }
  ],
  "segments": [
    {
      "id": "prose-and-action",
      "text": "That is the whole idea. Prose is narration…",
      "chapter": "the-manual-is-the-program",
      "start_ms": 18074,
      "duration_ms": 14357,
      "duration_source": "measured",
      "audio": "audio/prose-and-action.wav",
      "voice_source": "synthetic",
      "voice_source_actual": "synthetic",
      "downgrade_reason": null,
      "source_hash": "8c0fae65…",
      "audio_hash": "af7823e0…"
    }
  ]
}
```

Four fields that are easy to skip and worth wiring up:

- **`duration_source`** — `measured` from real audio, `estimated` from the
  word-count model. Render estimated segments with a visible marker during
  development; a re-dub will move every one of them.
- **`audio_hash`** — hash of the encoded file's bytes. Key a render cache on it
  and skip re-encoding unchanged segments. Repeated values are not a bug: the
  `null` backend emits silence, so equal-length segments hash equally.
- **`chapter`** — the slug of the chapter a segment was spoken in. Published
  because `chapters` omits chapters nobody speaks in, so reconstructing this
  from timestamps is lossy.
- **`words`** — per-word `start_ms`/`end_ms`, present only when the backend
  provides them. The key is absent rather than empty, so `segment.words &&`
  is a real test.

## Types

`src/manifest.ts`

```ts
export type Manifest = {
  manifest_version: number;
  script: string;
  locale: string;
  generated_by: string;
  duration_ms: number;
  audio: { format: string; sample_rate: number; channels: number };
  chapters: { id: string; title: string; start_ms: number }[];
  segments: Segment[];
};

export type Segment = {
  id: string;
  text: string;
  chapter: string;
  start_ms: number;
  duration_ms: number;
  duration_source: "measured" | "estimated";
  audio: string;
  voice_source: string;
  voice_source_actual: string;
  downgrade_reason: string | null;
  source_hash: string;
  audio_hash: string;
  words?: { text: string; start_ms: number; end_ms: number }[];
};

/** Milliseconds to frames. Only ever called on an absolute offset — see
 *  `Narrated` for why. */
export const frameAt = (ms: number, fps: number) => Math.round((ms * fps) / 1000);
```

## Composition

`src/Root.tsx`

```tsx
import { Composition, staticFile } from "remotion";
import { Narrated } from "./Narrated";
import type { Manifest } from "./manifest";

export const RemotionRoot = () => (
  <Composition
    id="Narrated"
    component={Narrated}
    fps={30}
    width={1920}
    height={1080}
    // Placeholder: calculateMetadata replaces it before anything renders.
    durationInFrames={1}
    defaultProps={{ manifestPath: "narration/en/narration.json", manifest: null }}
    calculateMetadata={async ({ props, defaultProps }) => {
      const path = props.manifestPath ?? defaultProps.manifestPath;
      const manifest: Manifest = await (await fetch(staticFile(path))).json();

      // A major version teleprompt did not promise is a hard stop, not a
      // warning: the fields below are exactly what changed.
      if (manifest.manifest_version !== 1) {
        throw new Error(`unsupported narration manifest v${manifest.manifest_version}`);
      }

      return {
        durationInFrames: Math.round((manifest.duration_ms * 30) / 1000),
        props: { ...props, manifest },
      };
    }}
  />
);
```

The composition's length comes from `duration_ms` and nowhere else. That is the
point of the integration: editing a paragraph changes the manifest, which
changes the video's length, without anyone touching the Remotion project.

`src/Narrated.tsx`

```tsx
import { AbsoluteFill, Audio, Sequence, staticFile, useVideoConfig } from "remotion";
import { frameAt, type Manifest } from "./manifest";
import { Caption } from "./Caption";

const DIR = "narration/en/";

export const Narrated = ({ manifest }: { manifest: Manifest }) => {
  const { fps } = useVideoConfig();

  return (
    <AbsoluteFill style={{ background: "#0b0d10" }}>
      {manifest.segments.map((segment) => {
        // Convert both absolute offsets to frames, then subtract. Two ways to
        // get this wrong, both of which look fine on a short script:
        //
        //   * rounding `duration_ms` on its own accumulates error and drifts
        //     the audio out of sync by the end of a long video;
        //   * taking the next segment's `start_ms` as this one's end clips the
        //     tail of the speech, because consecutive beats overlap wherever a
        //     transition was subtracted from the preceding one.
        //
        // A segment's own `start_ms + duration_ms` is authoritative.
        const from = frameAt(segment.start_ms, fps);
        const until = frameAt(segment.start_ms + segment.duration_ms, fps);

        return (
          <Sequence
            key={segment.id}
            name={segment.id}
            from={from}
            durationInFrames={until - from}
          >
            <Audio src={staticFile(DIR + segment.audio)} />
            <Caption segment={segment} />
          </Sequence>
        );
      })}
    </AbsoluteFill>
  );
};
```

`src/Caption.tsx`

```tsx
import { useCurrentFrame, useVideoConfig } from "remotion";
import type { Segment } from "./manifest";

export const Caption = ({ segment }: { segment: Segment }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  // Relative to this Sequence, which is what `words` offsets are relative to.
  const ms = (frame / fps) * 1000;

  return (
    <p style={{ position: "absolute", bottom: 80, padding: "0 120px", color: "#eef3f8" }}>
      {segment.words
        ? segment.words.map((w, i) => (
            <span key={i} style={{ opacity: ms >= w.start_ms ? 1 : 0.35 }}>{w.text} </span>
          ))
        : segment.text}
      {segment.duration_source === "estimated" && (
        <span title="not yet dubbed; this timing will move" style={{ color: "#febc2e" }}> ~</span>
      )}
    </p>
  );
};
```

## Keeping the two sides honest

```bash
teleprompt dub scripts/tour.md --out public/narration --check
```

Exit `3` when the committed manifest no longer matches the script, so a pull
request that edits prose without re-dubbing fails CI. Commit `narration.json`;
`audio/` is reproducible and can be gitignored, at the cost of needing a voice
backend wherever you render.

One sharp edge: `--check` writes nothing to `--out`, but it does synthesize
anything not already cached — it has to, or it would be comparing against
numbers it never measured. A first `--check` on a cold cache costs a full
render and needs a writable checkout.

## What the manifest does not carry

**Visual timing.** The manifest says when each *sentence* is spoken. It does not
say when each action span starts, how long the scheduler gave it, or which
policy governed it. A composition built on this can synchronise to speech but
not to action: every `hold`, `concurrent`, `stretch-action` and `trim-action`
decision stops at the boundary.

That data exists — `timelines/<script>.<locale>.json` holds it — but the
timeline is an internal review surface, versioned separately from the manifest
and explicitly not a contract. A consumer joining the two by document order is
building on something that may move under it.

Publishing a `beats` projection in the manifest is the fix, and it is tracked
rather than assumed. Until then, a Remotion project either draws its own
visuals against chapter and segment boundaries, or reads the timeline knowing
that it is unstable.

## Licence

teleprompt ships no Remotion code and takes no Remotion dependency. Remotion's
own licence — free for individuals and organisations up to three employees —
is between you and Remotion.
