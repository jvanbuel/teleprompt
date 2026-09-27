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
- GTK 4.14, libadwaita 1.5, VTE for GTK 4 (session mode's terminal),
  GStreamer with the plugins to play the captured clips, and ffmpeg (a
  session's microphone). On Ubuntu 24.04:

      sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev ffmpeg \
        libgstreamer1.0-dev \
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
| Ctrl+Shift+Space | record from the line you are on; during a take, keep it (during the count of three: cancel) |
| click a line | record from there |
| Ctrl+T | record from the top |
| Return | keep the take |
| p | pause or resume |
| m | mirror the text, for beam-splitter glass |
| + / − | text size |
| s | show or hide the monitor |
| Ctrl+Shift+S | the monitor in its own window, for a second display |

A take starts after a count of three; Settings turns that off.

## Drafting from a session

The app has a second mode for a script that doesn't exist yet: **Draft
from a session…** on the start page, or Ctrl+N. Name the new script, and a
terminal opens. Press Ctrl+Shift+Space to start recording, then talk
while you use the terminal, as if showing someone. Press it again, exit the
shell or click Stop to finish: `teleprompt record` drafts the script, with
what you said as its lines and what you typed as its tapes, and it opens in
the prompter to read back and re-take.

Ctrl+Shift+Space starts and stops recording in both modes. It isn't typed
into the terminal, and it isn't a desktop shortcut the way Ctrl+Space
(switching input source) is.

Set a punctuation model in Settings to give the draft sentences (see
`docs/guide/recording.md`). The microphone is the system's default
input, or ffmpeg's input in `TELEPROMPT_RECORD_MIC`. The server
stops with the app, even if the app is killed. The look is described in
`apps/DESIGN.md`.

## Tests

    cargo test                        # the API, launching, the state
    TELEPROMPT_BIN=… TELEPROMPT_MODEL=… cargo test   # and the real server
    TELEPROMPT_BIN=… TELEPROMPT_MODEL=… tests/ui.sh  # and the window
    TELEPROMPT_BIN=… TELEPROMPT_MODEL=… tests/session.sh  # and session mode

`tests/end_to_end.rs` launches the real server and reads it a recording of
`apps/fixtures/tour`. `tests/ui.sh` runs the app itself under Xvfb with that
recording as its microphone (`TELEPROMPT_MIC` takes any GStreamer source),
starts a take with Ctrl+T, keeps it with Return, and checks both lines were
kept, a clip was on screen, and the server went when the app was killed.
The API tests read `docs/api/v1/examples`, which the server is tested
against too.
