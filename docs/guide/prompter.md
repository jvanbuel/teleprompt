# Reading from a prompter

![The prompter mid-take: the next word in amber on the reading line, the shot playing in the monitor, the rundown, and the tally bar.](../images/prompter.png)

`teleprompt prompt <script>` shows the script's narration as a prompter that
follows your voice. It listens through the microphone, works out where you
are by matching what a local speech model hears against the script, and
keeps the next word highlighted a third of the way down the screen. Nothing
you say leaves the machine.

It runs in a browser, and as a native app: `apps/macos` in SwiftUI and
`apps/linux` in GTK. The apps launch `teleprompt prompt` themselves and play
the shots with the system's player; their READMEs say how to build them.
All of them need the recognizer and the model below.

## Setting it up

The recognizer is opt-in, because it downloads a native library (about
23 MB) when it builds:

```bash
cargo install --path crates/teleprompt-cli --features listen
```

It also needs a speech model, which teleprompt downloads only when you ask:

```bash
teleprompt setup speech-model --run
```

That is sherpa-onnx's streaming English model (Apache-2.0), unpacked into
`~/.local/share/teleprompt/models` (or `$TELEPROMPT_MODELS`), where
`prompt`, `record` and `import` find it. Without `--run`, `setup` prints the
command instead. To keep a model elsewhere, download and unpack it yourself
and pass its directory with `--model`. The smaller 20M model is cheaper but
misses the first words of a take.

## Reading

```bash
teleprompt prompt scripts/tour.md
```

Open the address it prints, press Ctrl+Shift+Space (⌘⇧Space on a Mac) to
start, and read. While you are on air, the glass has a red frame.
Everything you read is a take: press the key again to keep it, or Escape to
throw it away. Between takes, click any line to start a new take from
there, which is how you read one line again.

| key | does |
|---|---|
| Ctrl+Shift+Space (⌘⇧Space on a Mac) | start a take from the line you are on; during one, keep it |
| Ctrl+Enter (⌘Enter) | keep the take |
| Escape | throw the take away, or cancel the count before it |
| Ctrl+Z (⌘Z) | undo the take just kept: each line gets back the recording it replaced |
| click a line | between takes, start a new take from that line |
| p | pause and resume listening |
| w | review a line said in other words than it reads |
| m | mirror the text, for beam-splitter glass |
| + and − | text size |
| s | hide or show the screen |

It follows you, not a clock: stop, and it waits; misread a word, add an
aside or skip a sentence, and it keeps its place. It jumps ahead only when
several words in a row match further on, so a single word that happens to
occur later does not make the text lurch.

## The screen follows you

When the script has action blocks, the prompter shows their captured clips
beside the text, and starts each one when your voice gets to it. A marker in
the text shows where each shot starts:

| the block's policy | the shot starts |
|---|---|
| `hold` | when you finish the line above it |
| `concurrent` | on the line's first word, or on its `cue=` phrase |
| no line of its own | when you finish the line before it |

A shot the video would start part-way through its line (`align=end` or
`center`) starts on the word the plan puts it at.

Capture the clips first, with `teleprompt capture`. A shot without a clip
shows as missing, both in the text and on the screen. If you read on before
a clip ends, the next shot cuts it off, because the screen follows you and
not the clock. Press `s` to hide or show the screen.

Clips are H.264, which Chrome, Edge and Safari play. Chromium builds without
proprietary codecs cannot play them.

## Recording

When you keep a take, each line you read in full is saved as that line's
recording, `takes/<line>.wav`. The recording it replaces is put aside, in
`takes/.previous/`, and **Undo** on the message that says what was kept (or
Ctrl+Z) puts it back. A line you broke off or skipped is not saved. The take is cut between lines at the pause
after each one, so leave a breath between paragraphs. Lines with a
recording are marked at their left edge.

A recorded line is the line's voice from then on: `plan`, `dub`, `build`
and the preview use it, paced to its real length, and the lines without one
are synthesized. Every command names those. Edit a line and its recording
no longer matches it, so the line is synthesized until you read it again.

Recordings are source, like the script: commit `takes/`. The microphone is
recorded as it is, without the browser's noise suppression, so record
somewhere quiet.

## Keeping what you said

A line you read in full in other words than it reads, a word swapped or
left out, is still kept, and gets a blue dot beside its mark, with a
toast. Press W to see the difference: the words you didn't say struck
through, the ones you said instead in bold. **Use What I Said** rewords
the line in the script to match, as `teleprompt edit <script> said
<line>` does, and its recording stays current: nothing to read again.
**Keep the Script**, or Escape, leaves the line as written, to read
again; you aren't asked about it again until you reload the page. A small
speech model mishears, so a word a letter off, or heard as two ("a
round"), doesn't count as other words.
