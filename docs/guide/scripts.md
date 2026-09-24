# Writing scripts

A script is a Markdown file. Its paragraphs are what the narrator says, and
its fenced `teleprompt` blocks are what the screen does:

````markdown
---
teleprompt: 1
output:
  resolution: [1920, 1080]
  fps: 30
---

# Quick start

Every video here is built from a script you can read. {#welcome}

```teleprompt scene=terminal
Type "teleprompt plan scripts/tour.md"
Enter
Sleep 2s
```
````

- Front matter configures the script.
- Headings are chapters. Every line and block must be inside one.
- Each paragraph is a **line** of narration.
- Each `teleprompt` block is an **action**, written in the language of its
  scene (see [Scenes](scenes.md)).
- Anything else, such as lists, tables or other code blocks, is ignored, so
  a script can also be ordinary documentation.

Inline markup is read aloud as text: emphasis is dropped, and code spans
and links read as their contents.

## Line ids

`{#welcome}` at the end of a paragraph names the line. A line without an id
gets one from its position, `<chapter>-<n>`, which shifts when a paragraph
is inserted above it. The id names the line's audio file and its entry in
the timeline, so give ids to lines you'll keep editing.

## Pairing and policies

A line and the block right after it form one **item**. The block's
`policy=` decides how the two share time. Narration is never sped up,
slowed down or cut. Only the action adapts.

| policy | what happens |
|---|---|
| `hold` (default) | The line plays over the previous picture, then the action runs. |
| `concurrent` | The action runs during the line. `align=start` (default), `end` or `center` places the shorter one within the longer. |
| `stretch-action` | The action is re-timed to last exactly as long as the line, between `min_stretch` (0.33) and `max_stretch` (3.0) times its own pace. |
| `trim-action` | An action longer than its line is cut to the line's length, with a warning when it's more than `max_speedup` (2.0) times too long. |

Re-timing needs an adapter that can rewrite its source: tapes and
recordings can, while scripts and compositions take their line's length
anyway.

## Starting on a phrase

A concurrent action starts with its paragraph by default. `cue=` starts it
when the voice reaches a phrase instead:

````markdown
One command registers a server. flowrs config add asks for a name. {#config}

```teleprompt scene=terminal policy=concurrent cue="flowrs config add"
Type "flowrs config add"
Enter
```
````

When the voice backend timed its words (Kokoro with
`word_timings = true`, see [Voices](voices.md#word-timings)), the action
starts on the phrase's first word. Otherwise the start is interpolated from
where the phrase sits in the sentence, which lands within a syllable or
two. A cue naming something the line doesn't say is an error, and so is a
cue on any policy other than `concurrent`.

## Pauses and padding

`<!-- teleprompt: pause 800ms -->` on its own inserts a silent item. Every
line is padded by `lead_in` and `tail` (150 ms each by default), so speech
never runs into a transition.

## Transitions

```yaml
output:
  transition: { kind: crossfade, duration: auto, min_ms: 0, max_ms: 600 }
```

With `duration: auto`, the transition into the next item is half the
item's slack (how much longer the line is than its action), clamped to
`min_ms` and `max_ms`. It never exceeds the silence around the boundary,
so it never crossfades over speech. A fixed duration, such as
`duration: 400ms`, is used as written, with a warning when it overlaps
speech.

## Pronunciation

A synthetic voice reads spelling, so `MWAA` comes out as a word:

```yaml
voice:
  pronounce:
    MWAA: em-double-you-ay-ay
    TUI: tee-you-eye
```

This applies to synthesis only. The script, captions and manifest keep the
spelling. Matching is on whole words and case-sensitive, and correcting a
pronunciation re-renders only the lines that use that word.

## Attributes

Lines take attributes after their id, as in `{#intro voice.speed=1.1}`.
Blocks take them on the fence line. An unknown key is an error, and so is
a duration that doesn't parse or is longer than a day (`lead_in`, `tail`,
and a `pause`).

| on | keys |
|---|---|
| a line | `voice.source`, `voice.backend`, `voice.voice`, `voice.speed`, `lead_in`, `tail`, `lang` |
| a block | `scene`, `include`, `policy`, `align`, `cue`, `session`, `id`, `max_speedup`, `max_stretch`, `min_stretch`, `review` |

`include=path` takes the block's body from a file, so a tape or a
Playwright spec stays a real file its own tools can run.
`include=path#fragment` selects part of it, where the adapter supports that
(see [Recordings](scenes.md#recordings)).

## Configuration

Settings merge from these layers, with later ones winning:

1. built-in defaults
2. `teleprompt.toml` at the project root
3. the script's front matter
4. a chapter's ` ```yaml teleprompt ` block, immediately after its heading
5. line and block attributes
6. command-line flags

| section | keys | defaults |
|---|---|---|
| `voice` | `source`, `backend`, `voice`, `speed`, `pronounce` | `synthetic`, `null`, none, `1.0` |
| `timing` | `lead_in_ms`, `tail_ms`, `max_stretch`, `min_stretch`, `max_speedup` | `150`, `150`, `3.0`, `0.33`, `2.0` |
| `output` | `resolution`, `fps`, `transition` | `[1920, 1080]`, `30`, see above |
| `scene.<name>` | `adapter`, plus the adapter's own settings | `browser` → `playwright`, `terminal` → `vhs`, `media` → `media` |
| `backends.<id>` | the backend's own settings, only in `teleprompt.toml` | |

## Drafting from a document

```bash
teleprompt from README.md
```

`from` turns an existing Markdown document into a draft script. Prose
becomes lines, with ids assigned up front so the first edit can't shift
them. Shell code blocks become tapes that **type** the command and never
run it. They're marked `review=pending`, and `check` warns about them until
someone has read the tape and removed the attribute. Anything else, such as
a JSON payload or a TypeScript snippet, stays ordinary Markdown.

The result is a draft. No pacing is inferred and every block gets the
default policy, so choosing `concurrent` over `hold` is left to you. `from`
won't overwrite an existing file.

### From a Slidev deck

```bash
teleprompt from talk/slides.md --slidev --out scripts/talk.md
```

A deck's speaker notes are already what is said over each slide. `--slidev`
reads them the way Slidev does (a slide's notes are its last HTML comment)
and turns each into a paragraph followed by a block showing that slide.
Slidev's `[click]` markers split a note into one paragraph per click step:

```md
<!--
The deck does not change to be narrated.
[click] You can still present it with Slidev.
[click] You can still export it.
-->
```

This becomes three paragraphs over `2`, `2?clicks=1` and `2?clicks=2`.
`[click:3]` adds three clicks, as in Slidev. Slides are numbered as Slidev
numbers them: a `hide: true` or `disabled: true` slide has no number and its
notes are dropped with a warning, and a `src:` import contributes the
slides of the file it names (or the `#2-3` range of them). A slide with no
notes is left out, and `from` names it.
