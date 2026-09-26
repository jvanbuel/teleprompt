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

Set both in Settings (⌘,) the first time.

## Build

    ./bundle.sh            # Teleprompt.app, here
    open Teleprompt.app

`swift run Teleprompt` works too, but then macOS asks for the microphone on
behalf of your terminal.

## Keys

| key | does |
|---|---|
| click a line | record from there |
| Record | record from the line you are on |
| ⌘T | record from the top |
| return | keep the take (during the count of three: cancel) |
| space | pause or resume |
| m | mirror the text, for beam-splitter glass |
| + / − | text size |
| s | show or hide the monitor |
| ⌘⇧S | the monitor in its own window, for a second display |

A take starts after a count of three; Settings turns that off. The look is
described in `apps/DESIGN.md`; the typeface and the icon are bundled by
`bundle.sh`.

## Layout

- `Sources/TelepromptKit`: the API, launching the server, and the
  prompter's state. Foundation only; builds and is tested on Linux too.
- `Sources/Teleprompt`: the SwiftUI app, the microphone and the player.
- `Tests/TelepromptKitTests`: checks the API against
  `docs/api/v1/examples`, which the server is tested against too, and an
  end-to-end test that launches the real server and reads it a recording
  of `apps/fixtures/tour` (`TELEPROMPT_BIN` and `TELEPROMPT_MODEL`; see `EndToEndTests.swift`).
