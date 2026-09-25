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

It follows you, not a clock: stop, and it waits; misread a word, add an
aside or skip a sentence, and it keeps its place. It jumps ahead only when
several words in a row match further on, so a single word that happens to
occur later does not make the text lurch.

The prompter shows narration only. Recording what you read, and pacing the
screen to it, are not built yet.
