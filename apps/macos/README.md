# Teleprompt for macOS

A native prompter for teleprompt. Open a script and read: the text follows
your voice, each shot plays as you reach it, and every line you read in full
is kept as that line's take.

The app launches `teleprompt prompt` itself and talks to it over the
prompter API (`docs/design.md#prompter-api-version-1`). It needs:

- `teleprompt` built with the speech recognizer:
  `cargo install --path crates/teleprompt-cli --features listen`
- a speech model: download and unpack
  [sherpa-onnx-streaming-zipformer-en-2023-06-26](https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-2023-06-26.tar.bz2)

Set both in Settings (⌘,) the first time. To have the script's voice read
it instead (below), only the binary is needed, built with or without the
recognizer.

## Build

    ./bundle.sh            # Teleprompt.app, here
    open Teleprompt.app

`swift run Teleprompt` works too, but then macOS asks for the microphone on
behalf of your terminal.

## Keys

| key | does |
|---|---|
| ⌘⇧Space | record from the line you are on; during a take, keep it (during the count of three: cancel) |
| click a line | record from there |
| ⌘T | record from the top |
| return | keep the take |
| p | pause or resume |
| w | review a line said in other words than it reads: keep what you said, or the script |
| m | mirror the text, for beam-splitter glass |
| + / − | text size |
| s | show or hide the monitor |
| ⌘⇧S | the monitor in its own window, for a second display |
| F2 | reword the line you are on, where it stands (Prompter › Reword Line) |
| ⌥⌘Z | undo the last reword or instruction (Prompter › Undo Last Edit) |

A take starts after a count of three; Settings turns that off. The look is
described in `apps/DESIGN.md`; the typeface and the icon are bundled by
`bundle.sh`.

## Letting a voice read it

Choose **A voice reads** under "Who narrates" on the welcome page, and
the app runs `teleprompt prompt --voice`: the script is read by its voice
(the project's `[voice]`) rather than following yours, and no speech
model is needed. Play (Space, or ⌘⇧Space) reads from the line you are on,
lighting the words and playing the shots as it goes; Space or Escape
stops. Click a line for its panel: Listen, Read on from here, How to say
it (the line's `voice.instruct`), Say it again, and Reword. The margin
marks each line the voice reads with a waveform, faint until the voice
has made it. `docs/guide/prompter.md#letting-a-voice-read-it` has the
rest.

## Keeping what you said

A line read in full in other words than it reads gets a blue dot beside
its tick after the take, and a toast. W (or **Prompter › Keep What You
Said…**) shows the difference: the words not said struck through, the
ones said instead in bold. **Use What I Said** rewords the line in the
script, through the server, as `teleprompt edit <script> said <line>`
does, and its take stays current. **Keep the Script**, or Escape, leaves
it as written, and it isn't asked about again this session.

## Layout

- `Sources/TelepromptKit`: the API, launching the server, and the
  prompter's state. Foundation only; builds and is tested on Linux too.
- `Sources/Teleprompt`: the SwiftUI app, the microphone and the player.
- `Tests/TelepromptKitTests`: checks the API against
  `docs/api/v1/examples`, which the server is tested against too, and an
  end-to-end test that launches the real server and reads it a recording
  of `apps/fixtures/tour` (`TELEPROMPT_BIN` and `TELEPROMPT_MODEL`; see `EndToEndTests.swift`).
