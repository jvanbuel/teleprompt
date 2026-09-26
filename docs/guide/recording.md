# Recording a session

The quickest first draft of a terminal demo is to give it once: open a
shell, talk about what you're doing while you do it, and let teleprompt
turn that into a script.

```bash
teleprompt record scripts/tour.md --model path/to/model
```

`record` opens your shell and records it, keystroke by keystroke, together
with the microphone. Exit the shell when you're done. It then writes
`scripts/tour.md`:

- **What you said is the narration**, cut into lines wherever you paused.
  It's written out verbatim, so expect to edit it. The speech model hears
  no punctuation: pass `--punctuation <dir>` with the punctuation model
  below to get sentences, or each line is one long sentence.
- **What you typed is terminal tapes**, typed at the speed you typed, with
  your pauses kept and your Backspaces already applied. The `exit` that
  ended the session is left out.
- **Each tape runs where it happened.** A command you started while you
  were talking runs `concurrent` with that line, cued to the words you were
  saying when you started typing. A command you typed in a pause holds
  after the line before it.
- **Every line is spoken from your recording.** Each line's stretch of the
  recording is saved as its take in `takes/`, so `plan` and `build` use
  your voice from the start (see [takes](prompter.md)).

Here is what a short session becomes:

````markdown
# Tour

Let's see what is here.

```teleprompt scene=vhs policy=concurrent cue="what is"
Set TypingSpeed 60ms
Type "ls -la"
Enter
Sleep 700ms
```

And now the file.
````

Editing a line leaves its take behind, and the line is synthesized until
you record it again with [`prompt`](prompter.md). Editing a tape changes
nothing about the voice. The recording itself, `session.cast` and
`voice.wav`, is kept under `.teleprompt/traces/`, which is not committed.

## What it needs

- A build with the recognizer and a speech model, set up as for
  [`prompt`](prompter.md#setting-it-up).
- Optionally, sherpa-onnx's punctuation model (36 MB), for `--punctuation`:

  ```bash
  curl -LO https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-online-punct-en-2024-08-06.tar.bz2
  tar xjf sherpa-onnx-online-punct-en-2024-08-06.tar.bz2
  ```
- ffmpeg, which records the microphone. By default it uses the system's
  default input (PulseAudio on Linux, AVFoundation on macOS). Pass another
  as ffmpeg's input arguments with `--mic`, for example
  `--mic "-f alsa -i hw:1"`.
- A Unix terminal.

To record something other than your shell, put it after `--`:
`teleprompt record scripts/tour.md --model … -- bash --norc`.

## From a recording you already have

`import` does the second half on its own, from an asciicast with
keystrokes and a WAV of your voice:

```bash
asciinema rec --stdin session.cast   # asciinema 3: --capture-input
teleprompt import session.cast --voice voice.wav --model path/to/model
```

If the voice recording started later than the cast, say by how much with
`--offset-ms`. To use another recognizer, or a transcript you corrected,
pass its words as JSON with `--words` instead of `--model`:

```json
[{"text": "let's", "start_ms": 1000, "end_ms": 1350}, …]
```

The script goes to `scripts/<cast>.md` unless you name it with `--out`.
Neither command overwrites a script without `--force`, since that would
also replace its lines' takes.
