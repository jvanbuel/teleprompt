# Rendering

## Building a video

```bash
teleprompt build scripts/tour.md
```

`build` synthesizes the narration, publishes the manifest, captures any
scene that has no clip yet, and renders `build/tour.en.mp4`. Narration is
placed at the offsets the manifest published, each shot holds its
scheduled slot, and transitions overlap the way the scheduler granted.
`--resolution` and `--fps` override the script's `output:` block for one
render, and `--out` writes somewhere else. `teleprompt doctor` reports
whether this machine has an ffmpeg to render with.

`build` reads the manifest it just published, exactly as an outside
integrator does, so what you render and what you publish can't disagree.

A shot with no clip, because its scene can't be recorded here, holds its
slot as a slate. The timing is still right, and `build` says how many
slates it rendered.

### Captions

Every build writes subtitles beside the video, `build/tour.en.srt` and
`build/tour.en.vtt`, named so that players and upload pages pick them up.
They're the narration, as it's spoken:

- A cue holds at most two rows of 42 characters. A longer line is split,
  after a sentence or a comma where one falls far enough in.
- Each part of a line appears when its first word is said. With a voice
  that reports word timings, that's exact. With one that doesn't, the
  line's time is shared out by the length of each part.

### Names on screen

When someone in the cast first speaks, the video names them in the lower
third for four seconds, fading in and out. The name is their key
title-cased, `ada-lovelace` as `Ada Lovelace`, or what their `name` says,
which can say more:

```yaml
voices:
  charles-babbage:
    name: "Charles Babbage, inventor"
```

A narrator is named the same way with `voice.name`, and is otherwise not
named at all. `output.names: false` turns names off. They are drawn into
the chunks of picture they fall over, so a rebuild re-encodes only those
when a name changes.

### Chapters

A build also writes `build/tour.en.chapters.txt`, the script's headings
with where each starts, ready to paste into a YouTube description:

```
0:00 Introduction
1:12 Configuration
3:05 Deploying
```

YouTube shows chapters only when there are at least three, each at least
ten seconds long. When that isn't so, `build` says which rule fails.

### Rebuilds are incremental

The picture is encoded in pieces, and each piece is kept. A piece's key
covers everything that changes its frames: the clip, its length, the frame
size and rate, and the encoder settings. A rebuild copies every piece that
still matches rather than encoding it again. Rewording one sentence of the
manual re-encodes the picture around that sentence and copies the other
five and a half minutes, so 18.4 s becomes 4.2 s. A piece is keyed on its
length rather than its position, so a sentence that grows moves everything
after it without invalidating any of it. `--no-cache` encodes every frame.

The cache in `.teleprompt/cache/compose/` is capped: a build evicts the
least recently used pieces until what's left fits in 1 GB. Use
`--cache-max-mb` for a different budget, or `0` to keep nothing.
`teleprompt cache` reports what the caches hold, and
`teleprompt cache --prune-to-mb N` shrinks the video cache now. Everything
in it is derived, so evicting costs time and nothing else.

## Dubbing for another renderer

`teleprompt dub` writes the narration audio and a manifest, and renders no
video. A tool teleprompt doesn't control, such as Remotion, After Effects
or a web player, reads the manifest and owns the picture:

```bash
teleprompt dub scripts/tour.md --out public/narration
```

```
public/narration/en/narration.json
public/narration/en/audio/welcome.wav
public/narration/en/captions.srt
public/narration/en/captions.vtt
public/narration/en/chapters.txt
```

The manifest gives each line its id, text, absolute `start_ms` and
`duration_ms`, audio file and voice, and each shot its schedule. Deriving
your composition's length from `duration_ms` keeps narration in charge of
pacing on the other side too.

Two rules for consumers:

- A line lasts its **own** `duration_ms`. The next line's `start_ms` isn't
  this line's end, because actions and pauses leave gaps between lines and
  a fixed transition can make them overlap.
- Convert **absolute offsets** to frames and subtract, as
  `round(start_ms × fps / 1000)` and
  `round((start_ms + duration_ms) × fps / 1000)`. Rounding a duration on its
  own lets the error build up over a long video.

[Rendering with Remotion](../integrations/remotion.md) is a complete worked
example, with types for every field.

### Keeping it honest

```bash
teleprompt dub scripts/tour.md --out public/narration --check
```

This exits `3` when the committed manifest no longer matches the script,
so a pull request that edits prose without re-dubbing fails CI. Commit
`narration.json`. `audio/` can be regenerated and gitignored, at the cost
of needing a voice backend wherever you render.

`--check` writes nothing to `--out`, but it does synthesize anything not
yet cached, because it would otherwise compare against numbers it never
measured. A first `--check` on a cold cache costs a full dub and needs a
writable checkout.

## Watching an edit

```bash
teleprompt prompt --preview scripts/tour.md
```

The preview recompiles when you save, synthesizes only the lines whose text
changed, and opens on the first item that moved, with the ones that shifted
marked on the timeline strip. A reworded paragraph is audible about a second
later. The preview reads the same `narration.json` an outside consumer
does. A script that stops compiling shows its error while the last version
that compiled keeps playing.

## Environment variables

| variable | effect |
|---|---|
| `TELEPROMPT_REMOTION_BROWSER` | the headless Chrome Remotion renders with, unless `browser` is set in the scene |
| `TELEPROMPT_SLIDEV_BROWSER` | the browser Slidev exports with, unless `browser` is set in the scene |

The `TELEPROMPT_REQUIRE_*` variables are for the test suite; see
[CONTRIBUTING.md](../../CONTRIBUTING.md).
