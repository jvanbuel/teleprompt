# Reading from a prompter

`teleprompt prompt <script>` shows the script's narration as a prompter that
follows your voice. It listens through the microphone, works out where you
are by matching what a local speech model hears against the script, and
keeps the next word highlighted a third of the way down the screen. Nothing
you say leaves the machine.

## Setting it up

The recognizer is opt-in, because it downloads a native library (about
23 MB) when it builds:

```bash
cargo install --path crates/teleprompt-cli --features listen
```

teleprompt downloads nothing when it runs, so fetch the speech model once and
unpack it anywhere:

```bash
curl -LO https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-2023-06-26.tar.bz2
tar xjf sherpa-onnx-streaming-zipformer-en-2023-06-26.tar.bz2
```

That is sherpa-onnx's streaming English model. The smaller 20M model is
cheaper but misses the first words of a take.

## Reading

```bash
teleprompt prompt scripts/tour.md --model path/to/sherpa-onnx-streaming-zipformer-en-2023-06-26
```

Open the address it prints, click to start, and read. Each click starts a
new take from the top.

| key | does |
|---|---|
| space | pause and resume listening |
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

Recording what you read is not built yet.
