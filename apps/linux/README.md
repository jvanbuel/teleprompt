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
| r | record the lines reworded since their takes again, one after another |
| m | mirror the text, for beam-splitter glass |
| + / − | text size |
| s | show or hide the monitor |
| Ctrl+Shift+S | the monitor in its own window, for a second display |
| Ctrl+Z | undo the last timeline drag |
| Ctrl+Shift+C | capture the shots not yet captured, or changed since |
| Ctrl+B | capture, then build the video |

A take starts after a count of three; Settings turns that off.

## Rewording a line

Edit the script while it's open, in any editor: the app reloads it, and a
line reworded since its take gets an amber ring in the margin, since its
take no longer says what it says. Press R to record those lines again,
one after another: each is kept as you read past it, and the next starts.

## The timeline

Under the glass, the timeline shows the lines as the voice says them (green
once recorded) and the shots on screen, in time. Drag a shot onto a word of
its line to start it there, past the line to run it after, or onto another
line to move it; drag its end to stretch or shorten it. A chip says what a
drop will do before you let go. Each drop is written into the script with
`teleprompt edit`, and refused if the script would no longer compile;
Ctrl+Z undoes the last. Lines don't move: they're as long as the voice
says them. A shot that states no length of its own (a Playwright script,
a composition) moves but doesn't stretch.

A moved or stretched shot needs capturing again: Ctrl+Shift+C captures
what's missing, and Ctrl+B captures and builds the video, with its progress
across the top of the timeline. When it's built, the toast plays it.

## Drafting from a session

The app has a second mode for a script that doesn't exist yet: **Draft
from a session…** on the start page, or Ctrl+N. Name the new script, and a
terminal opens with the tools `teleprompt record --tools` found: asciinema
or VHS for the terminal, Playwright for a browser, which opens its own
window. Press Ctrl+Shift+Space to start recording, then talk while you
work, as if showing someone. Press it again, exit the shell (or close the
browser) or click Stop to finish: `teleprompt record` drafts the script,
with what you said as its lines and the recording between them, and it
opens in the prompter to read back and re-take.

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
