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
- Headings are chapters. Every line and block must be inside one. A
  heading can end in settings for its chapter, as in
  `# The interview {speaker=guest voice.speed=1.1}`: see
  [Configuration](#configuration).
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

## Speakers

A script can have more than one voice. Name the cast once, in
`teleprompt.toml` or the front matter, each speaker's voice as it differs
from `[voice]`, the narrator's:

```toml
[voice]
backend = "kokoro"
voice = "af_heart"

[voices.guest]
backend = "gemini"
voice = "Puck"
instruct = "dry, a little deadpan"
```

Then say who speaks a line the way a transcript does, with their name in
bold before what they say:

```md
Welcome back. Today I'm talking to someone who deploys on Fridays. {#intro}

**Guest:** Only on Fridays, actually. {#friday}

**Guest:** Because nobody's watching. {#because voice.instruct=conspiratorial}
```

The label is who, not what: it isn't said, and it matches the cast
whatever its case, with spaces for dashes: `**Guest:**` is
`[voices.guest]`, and `**Ada Lovelace:**` is `[voices.ada-lovelace]`. `**Guest**:`
works too. A line with no label is the narrator's. A speaker's voice sits
over the chapter's settings and under the line's own, so the last line
keeps the guest's voice with its own delivery. `speaker: guest` in the
front matter, or `{speaker=guest}` on a chapter's heading, makes a speaker
the default there. A label never carries over from one line to the next,
so moving a line never changes who says the lines after it.

When the speaker changes, the new one waits `timing.turn_gap_ms` (250)
before speaking, on top of their line's `lead_in`, the way someone does
before answering. Set it to `0` for a quick back-and-forth, in a
chapter's heading for one exchange: `# Rapid fire {timing.turn_gap_ms=0}`.

A bold label naming nobody in the cast, such as `**Note:**`, is only
text, and is said like the rest. When the script has a cast, `check`
warns about one, since it is more likely a typo than something to say. Speakers may use different backends: each line is
spoken by its own, and `dub` converts every line to the first line's rate
and channels, since the manifest has one audio format.

`check` fails on a default `speaker` not in the cast, naming who is. The manifest
gives each line's `speaker`, and WebVTT captions put a speaker's cues in
their voice span (`<v guest>`). A line you record plays from your take
whoever its speaker is, so `@me` with a voice of yours to fall back on is
a table read: you record your lines, and the cast reads the rest.

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
4. a chapter's settings: on its heading, as in `# Setup {#setup
   voice.speed=1.1}`, with a setting's path dotted; or, for more than
   fit there, a ` ```yaml teleprompt ` block immediately after the
   heading, which wins over the heading's. The heading's `#id` is the
   chapter's, which its lines' ids start with, so it can be renamed
   without renumbering them.
5. line and block attributes
6. command-line flags

| section | keys | defaults |
|---|---|---|
| `voice` | `backend`, `voice`, `speed`, `instruct`, `pronounce` | `null`, none, `1.0`, none |
| `voices.<speaker>` | the same keys as `voice`, over it for that speaker's lines, and `name`, as the video [names them](rendering.md#names-on-screen) | |
| `speaker` | who says a line with no label | the narrator |
| `timing` | `lead_in_ms`, `tail_ms`, `turn_gap_ms`, `max_stretch`, `min_stretch`, `trim_warn_above`, `min_line_speed`, `max_line_speed`, `min_take_speed`, `max_take_speed`, `length_ms` | `150`, `150`, `250`, `3.0`, `0.33`, `2.0`, `0.9`, `1.15`, `0.95`, `1.08`, none |
| `output` | `resolution`, `fps`, `transition`, `names` | `[1920, 1080]`, `30`, see above, `true` |
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

### From a transcript

```bash
teleprompt from interview.vtt --out scripts/interview.md
teleprompt from interview.txt --transcript --out scripts/interview.md
```

The transcript of a conversation becomes a script with a line per turn,
each opening with who says it, and everyone in it the cast:

```md
---
teleprompt: 1
voices:
  ada-lovelace: {}
  charles-babbage: {}
---

# Interview

**Ada Lovelace:** The engine weaves algebraic patterns. {#the-engine-weaves}

**Charles Babbage:** Just as the loom weaves flowers. {#just-as-the}
```

Captions are read by their extension, `.vtt` and `.srt`: a WebVTT voice
span (`<v Ada Lovelace>`) names the speaker, and so does `Name:` opening a
cue. A speaker's run of cues is one line until it passes 40 words, then it
goes on in another from the next cue that ends a sentence. `--transcript`
reads text, a paragraph a line, in the shapes transcripts come in:
`Name: …`, `**Name:** …`, a timestamp before either (`[00:01:02] Name:`),
or a line of its own naming the speaker and when (`Ada Lovelace  0:03`),
as meeting tools export them. A paragraph naming no one goes on with who
spoke last.

Each speaker reads in the narrator's voice until you give them one, so
give each a `voice` under `voices` next: the draft's front matter says
where.

#### Keeping their own voices

```bash
teleprompt from interview.vtt --audio interview.m4a --out scripts/interview.md
```

With the conversation's recording, each line is given its stretch of it as
its take, from when its captions say it starts to when they say it ends,
with a little either side and never past halfway to the next line. Every
line then speaks in the voice of whoever said it, and the video is the
conversation, edited as text: delete a line, move it, put a narrator's
line between two. A line you reword no longer matches its take, so it is
spoken by its speaker's voice from the cast instead, and the prompter puts
it in the retake queue. To re-voice someone entirely, say a guest who
would rather not be heard, pass `--revoice guest`, and their lines are
left to their voice in the cast.

A WAV is read as it is, and anything else through ffmpeg. Captions say
when each turn starts and ends; a text transcript needs a time on every
turn (`[00:01:02] Ada:` or `Ada  1:02`), and each runs until the next.
The draft has to go into a project, which keeps the takes.
