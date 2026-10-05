# Drafting a script

A script need not start empty. Teleprompt drafts one from a session you
give once, from a document or a deck you already have, or from a
conversation. Each draft is a script to read through and edit, never a
finished one.

## From a session

The quickest first draft of a demo is to give it once: work in a terminal
or a browser, talk about what you're doing while you do it, and let
teleprompt turn that into a script.

```bash
teleprompt record scripts/tour.md
```

`record` hands the recording to a tool you already use, and records the
microphone beside it. Pick the tool with `--with`:

| `--with` | Records | You finish by | Timing |
|---|---|---|---|
| `asciinema` (default) | your shell, with `asciinema rec` | exiting the shell | exact |
| `vhs` | your shell, with `vhs record` | exiting the shell | close: rebuilt from the tape's `Sleep`s |
| `playwright` | a browser, with `playwright codegen` | closing its window | when each step appeared in its script |

`teleprompt record --tools` lists them and says which are installed. When
you're done it writes `scripts/tour.md`, and the recording beside it in
`scripts/recordings/`:

- **What you said is the narration**, cut into lines wherever you paused.
  It's written out verbatim, so expect to edit it. The speech model hears
  no punctuation: pass `--punctuation <dir>` with the punctuation model
  below to get sentences, or each line is one long sentence.
- **What you did is the recording itself**, in the tool's own format: a
  cast, a tape, a Playwright script. It's cut into parts between the lines,
  with the tool's own marks, and the script includes each part where it
  happened with `include=file#part`. The `exit` that ended the session is
  left out.
- **Each part runs where it happened.** A command you started while you
  were talking runs `concurrent` with that line, cued to the words you were
  saying when you started. One you gave in a pause holds after the line
  before it.
- **Every line is spoken from your recording.** Each line's stretch of the
  recording is saved as its take in `takes/`, so `plan` and `build` use
  your voice from the start (see [takes](prompter.md)).

Here is what a short session becomes:

````markdown
# Tour

Let's see what is here.

```teleprompt scene=asciinema include=recordings/tour.cast#1 policy=concurrent cue="what is"
```

And now the file.

```teleprompt scene=asciinema include=recordings/tour.cast#2
```
````

An asciinema part is played back exactly as it was recorded; a VHS or
Playwright part runs again when the video is built. Editing a line leaves
its take behind, and the line is synthesized until you record it again
with [`serve`](prompter.md). Editing the recording, or where a block's
`include=` points, changes nothing about the voice. The raw recordings and
`voice.wav` are kept under `.teleprompt/traces/`, which is not committed.

### What it needs

- A build with the recognizer and a speech model, set up as for
  [`serve`](prompter.md#setting-it-up).
- The tool you record with: [asciinema](https://asciinema.org) 2 or 3,
  [VHS](https://github.com/charmbracelet/vhs), or Playwright (found with
  `npx`). `teleprompt setup asciinema` (or `vhs`, `playwright`) says what is
  missing and how to install it.
- Optionally, sherpa-onnx's punctuation model (36 MB), which gives the
  narration capitals and punctuation: `teleprompt setup punctuation-model
  --run` installs it where `record` and `import` find it, or pass one with
  `--punctuation`.
- ffmpeg, which records the microphone. By default it uses the system's
  default input (PulseAudio on Linux, AVFoundation on macOS). Pass another
  as ffmpeg's input arguments with `--mic`, for example
  `--mic "-f alsa -i hw:1"`.
- A Unix terminal.

The Linux app records sessions too: **Draft from a session…** opens a
terminal and offers the tools that are installed, Ctrl+Shift+Space starts
and stops recording, and the draft opens in the prompter when you stop.

To record a terminal tool with something other than your shell, put it
after `--`: `teleprompt record scripts/tour.md -- bash --norc`.
To start a browser on a page, pass `--url`.

### From a recording you already have

`import` does the second half on its own, from a recording and a WAV of
your voice:

```bash
asciinema rec --stdin session.cast   # asciinema 3: --capture-input
teleprompt import session.cast --voice voice.wav
```

The tool is told by the extension (`.cast`, `.tape`), or name it with
`--with`. A Playwright script says nothing about when each step happened,
so one can only be drafted by `record`. If the voice recording started
later than the session, say by how much with `--offset-ms`. To use another
recognizer, or a transcript you corrected, pass its words as JSON with
`--words` instead of `--model`:

```json
[{"text": "let's", "start_ms": 1000, "end_ms": 1350}, …]
```

The script goes to `scripts/<recording>.md` unless you name it with
`--out`. Neither command overwrites a script or its recording without
`--force`, since that would also replace its lines' takes.

## From a document

```bash
teleprompt import README.md
```

`import` turns an existing Markdown document into a draft script. Prose
becomes lines, with ids assigned up front so the first edit can't shift
them. Shell code blocks become tapes that **type** the command and never
run it. They're marked `review=pending`, and `check` warns about them until
someone has read the tape and removed the attribute. Anything else, such as
a JSON payload or a TypeScript snippet, stays ordinary Markdown.

The result is a draft. No pacing is inferred and every block gets the
default policy, so choosing `concurrent` over `hold` is left to you. `import`
won't overwrite an existing file.

## From a Slidev deck

```bash
teleprompt import talk/slides.md --slidev --out scripts/talk.md
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
notes is left out, and `import` names it.

## From a transcript

```bash
teleprompt import interview.vtt --out scripts/interview.md
teleprompt import interview.txt --transcript --out scripts/interview.md
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

### Keeping their own voices

```bash
teleprompt import interview.vtt --audio interview.m4a --out scripts/interview.md
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

A transcript's words are what someone heard, and some will be wrong.
Correcting one changes the line, which would set its take aside for
recording again; `teleprompt edit scripts/interview.md keep <line>` says
the take still says it, and `keep --all` does so for every line you have
corrected.

## From a conversation's recording

```bash
teleprompt setup conversations --run
teleprompt import interview.m4a --out scripts/interview.md
```

With no transcript at all, `import` transcribes the recording itself, on
your machine, and tells the voices in it apart: each turn becomes a line
labelled `**Speaker 1:**`, `**Speaker 2:**` in the order they first speak,
each speaking its stretch of the recording as its take. A turn begins and
ends where its voice does, so a take neither clips its first word nor
carries the next speaker's. `--speakers 2` says how many there are, which
tells them apart better than guessing; `--revoice` works as with
`--audio`.

`setup conversations` installs the three models it uses: the speech
model (310 MB) to transcribe, the punctuation model (31 MB) for capitals
and full stops, and the speaker models (47 MB) to tell the voices apart.
Run in a terminal without them, `import` offers to install them.

Rename the speakers in the cast, `speaker-1: { name: "Ada Lovelace" }`,
which is how the video names them, then read the draft through: the
transcription will have misheard some words, and `edit keep` keeps the
takes of the lines you correct. Without the speaker models every line is
the narrator's, and `import` says so. This needs the build with speech
models (`--features listen`), as `serve` and `import` do.
