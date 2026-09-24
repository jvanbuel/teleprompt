# Rendering a teleprompt script with Remotion

teleprompt publishes narration and its timing; Remotion owns the picture. This
is the whole integration, and the seam between them is one file:
`narration.json`.

> This page is the route where Remotion owns the *whole* video. Where
> teleprompt should own the video and Remotion draw some of its scenes,
> use a `scene=` block whose adapter is `remotion` instead: it renders
> your project's own compositions by id — see
> [Motion graphics](../guide/scenes.md#motion-graphics) and
> `examples/remotion`.

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
no Remotion dependency and the workspace ships no Node, which is deliberate.
The code here is copy-paste material, kept short enough to audit by eye. If it
drifts from the manifest, the manifest is right and this document is wrong —
`crates/teleprompt-compile/src/manifest.rs` is the source of truth, and
`manifest_version` moves when its shape does.

## What the manifest gives you

```jsonc
{
  "manifest_version": 2,
  "script": "tour.md",
  "locale": "en",
  "generated_by": "teleprompt 0.1.0",
  "duration_ms": 322730,
  "audio": { "format": "wav", "sample_rate": 24000, "channels": 1 },
  "chapters": [
    { "id": "the-manual-is-the-program", "title": "The manual is the program", "start_ms": 150 }
  ],
  "lines": [
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
  ],
  "shots": [
    {
      "shot": "prose-and-action-a#0",
      "line": "prose-and-action",
      "scene": "terminal",
      "adapter": "vhs",
      "start_ms": 32581,
      "duration_ms": 2630,
      "duration_source": "exact",
      "policy": "hold",
      "transition": { "kind": "crossfade", "duration_ms": 300 },
      "shot_hash": "c590e04a…",
      "capture_key": "cc98e507…"
    }
  ]
}
```

Four fields that are easy to skip and worth wiring up:

- **`duration_source`** — `measured` from real audio, `estimated` from the
  word-count model. Render estimated lines with a visible marker during
  development; a re-dub will move every one of them.
- **`audio_hash`** — hash of the encoded file's bytes. Key a render cache on it
  and skip re-encoding unchanged lines. Repeated values are not a bug: the
  `null` backend emits silence, so equal-length lines hash equally.
- **`chapter`** — the slug of the chapter a line was spoken in. Published
  because `chapters` omits chapters nobody speaks in, so reconstructing this
  from timestamps is lossy.
- **`words`** — per-word `start_ms`/`end_ms`, present only when the backend
  provides them. The key is absent rather than empty, so `line.words &&`
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
  lines: Line[];
  shots: Shot[];
};

export type Shot = {
  shot: string;
  /** The line spoken over this shot, or null for one after a mark. */
  line: string | null;
  scene: string;
  adapter: string;
  start_ms: number;
  duration_ms: number;
  duration_source: "exact" | "measured" | "estimated" | "unknown";
  policy: "hold" | "concurrent" | "stretch-action" | "trim-action";
  transition: { kind: string; duration_ms: number };
  shot_hash: string;
  capture_key: string;
  /** Present only for a block with `session="…"`. */
  session?: string;
};

export type Line = {
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
      if (manifest.manifest_version !== 2) {
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
      {manifest.lines.map((line) => {
        // Convert both absolute offsets to frames, then subtract. Two ways to
        // get this wrong, both of which look fine on a short script:
        //
        //   * rounding `duration_ms` on its own accumulates error and drifts
        //     the audio out of sync by the end of a long video;
        //   * taking the next line's `start_ms` as this one's end stretches a
        //     line across the gap an action or pause leaves after it, and
        //     clips it where a fixed transition makes two lines overlap.
        //
        // A line's own `start_ms + duration_ms` is authoritative.
        const from = frameAt(line.start_ms, fps);
        const until = frameAt(line.start_ms + line.duration_ms, fps);

        return (
          <Sequence
            key={line.id}
            name={line.id}
            from={from}
            durationInFrames={until - from}
          >
            <Audio src={staticFile(DIR + line.audio)} />
            <Caption line={line} />
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
import type { Line } from "./manifest";

export const Caption = ({ line }: { line: Line }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  // Relative to this Sequence, which is what `words` offsets are relative to.
  const ms = (frame / fps) * 1000;

  return (
    <p style={{ position: "absolute", bottom: 80, padding: "0 120px", color: "#eef3f8" }}>
      {line.words
        ? line.words.map((w, i) => (
            <span key={i} style={{ opacity: ms >= w.start_ms ? 1 : 0.35 }}>{w.text} </span>
          ))
        : line.text}
      {line.duration_source === "estimated" && (
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

## Placing the picture

`shots` is the other half: one entry per scheduled action shot, carrying the
numbers the **scheduler** arrived at rather than the ones the adapter proposed.
That difference is the whole point — it is what separates replaying a tape at
its authored pace from replaying it at the pace its narration bought.

```tsx
{manifest.shots.map((shot) => {
  const from = frameAt(shot.start_ms, fps);
  const until = frameAt(shot.start_ms + shot.duration_ms, fps);
  return (
    <Sequence key={shot.shot} name={shot.shot} from={from} durationInFrames={until - from}>
      <Scene shot={shot} />
    </Sequence>
  );
})}
```

Same arithmetic as a line, for the same reasons. A few notes on the fields:

- **`line`** is `null` for a shot that follows a mark inside a block — the
  narration belongs to the block's first shot, and the rest run under whatever
  the policy left of it. It is also `null` for a pause.
- **`scene: "pause"`** is a shot during which the picture holds. Skipping it
  runs the next shot early.
- **`duration_source: "exact"`** means the adapter's language states the
  shot's timing in full. A tape containing `Wait` reports `estimated`
  instead, bounded by its timeout, and a Playwright script reports
  `unknown` and takes its line's length.
- **`transition`** is the outgoing transition as scheduled. Deriving gaps from
  neighbouring offsets instead is the arithmetic that goes wrong exactly where
  items overlap.
- **`shot_hash`** identifies the shot's own source, the counterpart of a
  line's `audio_hash`. **`capture_key`** identifies its *picture*: its source
  and every shot before it in the same session, since the same keystroke can
  show two different screens. Key a per-shot render cache on `capture_key`.

What the manifest does not carry is the shot's **source** — the tape or the
Playwright script. A consumer that draws its own picture does not need it;
one that wants to replay the adapter's own actions does, and that is the
renderer's business rather than an integrator's.

## Licence

This route ships no Remotion code and takes no Remotion dependency. The
`remotion` scene adapter is the other direction — teleprompt owns the video
and renders the project's compositions into it — and it too uses the
project's own Remotion install rather than bundling one. Remotion's own
licence — free for individuals and organisations up to three employees —
is between you and Remotion.
