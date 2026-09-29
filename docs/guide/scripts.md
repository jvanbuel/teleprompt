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

```teleprompt scene=vhs
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
`policy=` decides how the two share time, and which of them leads. The
line leads unless you say otherwise: it's as long as it takes to say, and
the action adapts. Narration is never cut, and only `fit-line` changes its
speed.

| policy | what happens |
|---|---|
| `hold` (default) | The line plays over the previous picture, then the action runs. |
| `concurrent` | The action runs during the line. `align=start` (default), `end` or `center` places the shorter one within the longer. `align` means nothing to the other policies, so there it's an error. |
| `fit-action` | The action is sped up or slowed down to last exactly as long as the line, between `min_stretch` (0.33) and `max_stretch` (3.0) times its own pace. |
| `trim-action` | An action longer than its line is cut to the line's length, with a warning when it's more than `trim_warn_above` (2.0) times too long. |
| `fit-line` | The picture leads: it keeps its length, and the line is played faster or slower to fit it. See [below](#led-by-the-picture). |

Re-timing needs an adapter that can rewrite its source: tapes and
recordings can, while scripts and compositions take their line's length
anyway.

`stretch=` re-times the action as you say rather than to its line:
`stretch=1.5` runs its shots half as long again, `stretch=0.8` a fifth
faster, between `min_stretch` and `max_stretch`. It goes with `hold` and
`concurrent`; `fit-action`, `trim-action` and `fit-line` set the pace
themselves. `stretch=` never touches the narration, only the action, and
only one that states its own length.

## Led by the picture

Some pictures have a length you can't change: a recorded terminal
session, an animation, a clip cut to music. `policy=fit-line` makes the
picture lead:

````markdown
Deployment is one command, and it streams progress as it goes. {#deploy}

```teleprompt scene=cast policy=fit-line
select #3
```
````

The item is as long as the picture, and the line is played at the tempo
that fits it, keeping its pitch: between `min_line_speed` (0.9) and
`max_line_speed` (1.15) times its own pace, or `min_take_speed` (0.95) and
`max_take_speed` (1.08) for a line you recorded, since a real voice sounds
stretched sooner. Past a bound, `check` warns and says about how many
words to cut:

```
warning: deploy: the line runs 4900ms against the 3700ms its picture leaves:
         1.32x, past max_line_speed 1.15; cut about 2 of its 11 words
```

A picture must state its length for this. A shot that doesn't, such as a
Playwright script, takes one with `budget=`: `policy=fit-line budget=6.5s`.
`cue=` and `align=` place an action against its line, so they're errors
with `fit-line`. A script can mix both kinds of items freely: a slide held
while you speak, then a recording your next line has to fit.

For a video with a set length, say 60 seconds, set `timing.length_ms:
60000`. When the timeline runs over, `check` says by how much, how much of
it is held by `fit-line` pictures (which rewording can't shorten), about
how many words of the rest to cut, and how long each chapter runs.

## Editing from a timeline

`teleprompt edit` makes the edits a timeline drag does, as the attributes
you would write: `cue <block> --word N` starts a block on a word of its
line, `hold <block>` runs it after the line, `move <block> --after <line>`
pairs it with another line, and `stretch <block> --by F` scales its
stretch. Blocks and lines are named by their ids in `plan`. Nothing is
written if the script would then not compile. The Linux app's timeline
drags shots with it, with Ctrl+Z to undo; it is left out of `teleprompt
--help` for that reason, since by hand the attributes are quicker.

## Starting on a phrase

A concurrent action starts with its paragraph by default. `cue=` starts it
when the voice reaches a phrase instead:

````markdown
One command registers a server. flowrs config add asks for a name. {#config}

```teleprompt scene=vhs policy=concurrent cue="flowrs config add"
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

## Lines that are hard to say

`teleprompt check` also reads the narration the way a listener hears it,
and warns about:

- **A sentence of more than 30 words.** It's hard to say in one breath,
  and hard to read as captions. Split it.
- **Words a voice reads as code:** file names (`tour.md`), paths, URLs,
  `--flags`, `snake_case` and `camelCase` names. A voice may spell them
  out. Reword the line, or say how under `pronounce`, which silences the
  warning.
- **A doubled word**, like "the the". ("that that" and "had had" are
  English.)

These are warnings: `check` still passes. With `--locale`, the translated
lines are checked.

## Attributes

Lines take attributes after their id, as in `{#intro voice.speed=1.1}`.
Blocks take them on the fence line. An unknown key is an error, and so is
a duration that doesn't parse or is longer than a day (`lead_in`, `tail`,
and a `pause`). The same limit applies to the `_ms` settings in
`teleprompt.toml` and front matter.

| on | keys |
|---|---|
| a line | `voice.backend`, `voice.voice`, `voice.speed`, `voice.instruct`, `lead_in`, `tail`, `lang` |
| a block | `scene`, `include`, `policy`, `align`, `cue`, `session`, `id`, `stretch`, `budget`, `trim_warn_above`, `max_stretch`, `min_stretch`, `review` |

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
| `voice` | `source`, `backend`, `voice`, `speed`, `instruct`, `pronounce` | `synthetic`, `null`, none, `1.0`, none |
| `timing` | `lead_in_ms`, `tail_ms`, `max_stretch`, `min_stretch`, `trim_warn_above`, `min_line_speed`, `max_line_speed`, `min_take_speed`, `max_take_speed`, `length_ms` | `150`, `150`, `3.0`, `0.33`, `2.0`, `0.9`, `1.15`, `0.95`, `1.08`, none |
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
