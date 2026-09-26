# Teleprompt for Linux

![The prompter mid-take](../../docs/images/prompter.png)

A native prompter for teleprompt, in GTK 4 and libadwaita. Open a script
and read: the text follows your voice, each shot plays as you reach it, and
every line you read in full is kept as that line's take.

The app launches `teleprompt prompt` itself and talks to it over the
prompter API (`docs/design.md#prompter-api-version-1`). It needs:

- `teleprompt` built with the speech recognizer:
  `cargo install --path crates/teleprompt-cli --features listen`
- a speech model: download and unpack
  [sherpa-onnx-streaming-zipformer-en-2023-06-26](https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-2023-06-26.tar.bz2)
- GTK 4.14, libadwaita 1.5 and GStreamer, with the plugins to play the
  captured clips. On Ubuntu 24.04:

      sudo apt install libgtk-4-dev libadwaita-1-dev libgstreamer1.0-dev \
        libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-good \
        gstreamer1.0-libav libgtk-4-media-gstreamer

Set the binary and the model in Settings (Ctrl+,) the first time, or with
`TELEPROMPT_BIN` and `TELEPROMPT_MODEL`, which override the settings.

## Build and run

    cargo run --release -- path/to/script.md

To install it with a launcher entry:

    cargo install --path apps/linux
    install -Dm644 apps/linux/io.github.jvanbuel.Teleprompt.desktop \
      ~/.local/share/applications/io.github.jvanbuel.Teleprompt.desktop
    install -Dm644 apps/icons/teleprompt.svg \
      ~/.local/share/icons/hicolor/scalable/apps/io.github.jvanbuel.Teleprompt.svg

## Keys

| key | does |
|---|---|
| click a line | record from there |
| Record button | record from the line you are on |
| Ctrl+T | record from the top |
| Return | keep the take (during the count of three: cancel) |
| space | pause or resume |
| m | mirror the text, for beam-splitter glass |
| + / − | text size |
| s | show or hide the monitor |
| Ctrl+Shift+S | the monitor in its own window, for a second display |

A take starts after a count of three; Settings turns that off. The server
stops with the app, even if the app is killed. The look is described in
`apps/DESIGN.md`.

## Tests

    cargo test                        # the API, launching, the state
    TELEPROMPT_BIN=… TELEPROMPT_MODEL=… cargo test   # and the real server
    TELEPROMPT_BIN=… TELEPROMPT_MODEL=… tests/ui.sh  # and the window

`tests/end_to_end.rs` launches the real server and reads it a recording of
`apps/fixtures/tour`. `tests/ui.sh` runs the app itself under Xvfb with that
recording as its microphone (`TELEPROMPT_MIC` takes any GStreamer source),
starts a take with Ctrl+T, keeps it with Return, and checks both lines were
kept, a clip was on screen, and the server went when the app was killed.
The API tests read `docs/api/v1/examples`, which the server is tested
against too.
