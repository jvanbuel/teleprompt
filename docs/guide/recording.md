# Recording a session

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

## What it needs

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

## From a recording you already have

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
