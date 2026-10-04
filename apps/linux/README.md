# Teleprompt for Linux

![The prompter mid-take](../../docs/images/prompter.png)

Teleprompt's prompter in a window of its own, in GTK 4 and libadwaita.
Open a script and read: the text follows your voice, each shot plays as you
reach it, and every line you read in full is kept as that line's take.

The prompter is the page `teleprompt serve` serves, the same page a
browser shows, and so is setting teleprompt up. The app opens on a
welcome page of its own: open a script, one of those opened last, or
draft one from a session, and who narrates. It launches the server
itself in the script's project, shows its page in WebKit, gives it the
microphone, and opens its screen in a second window when it asks.
Around it the app has settings and session mode, which drafts a script
from a terminal session. What the
prompter does, and its keys, are in `docs/guide/prompter.md`; `?` lists
the keys in the app.

It needs:

- `teleprompt` built with the speech recognizer:
  `cargo install --path crates/teleprompt-cli --features listen`
- a speech model: `teleprompt setup speech-model --run` installs
  sherpa-onnx's streaming English model where teleprompt finds it, or the
  page offers to when you open a script
- GTK 4.14, libadwaita 1.5, WebKitGTK 6.0, VTE for GTK 4 (session mode's
  terminal), GStreamer's plugins to play the captured clips and hear the
  microphone, and ffmpeg (a session's microphone). On Ubuntu 24.04:

      sudo apt install libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev \
        libvte-2.91-gtk4-dev ffmpeg gstreamer1.0-plugins-good gstreamer1.0-libav

Set the binary in Settings (Ctrl+,) the first time if it isn't where
`cargo install` puts it. The model is the one `teleprompt setup` installed
unless Settings names another. `TELEPROMPT_BIN` and `TELEPROMPT_MODEL`
override both.

## Build and run

    cargo run --release -- path/to/script.md

To install it with a launcher entry:

    cargo install --path apps/linux
    install -Dm644 apps/linux/io.github.jvanbuel.Teleprompt.desktop \
      ~/.local/share/applications/io.github.jvanbuel.Teleprompt.desktop
    install -Dm644 apps/icons/teleprompt.svg \
      ~/.local/share/icons/hicolor/scalable/apps/io.github.jvanbuel.Teleprompt.svg

## The app's keys

| key | does |
|---|---|
| Ctrl+O | open a script |
| Ctrl+Shift+O | the welcome page, with the scripts opened last |
| Ctrl+N | draft a script from a session |
| Ctrl+E | open the script in your own editor |
| Ctrl+, | settings |
| Ctrl+? | these keys |
| Ctrl+Shift+Space | in session mode, start recording, and stop and draft |

The prompter's keys are the page's, the same as in a browser. A take
starts after a count of three; Settings turns that off.

## Letting a voice read it

Choose **A voice reads** under "Who narrates" on the welcome page, and the
script opens read by its voice rather than following yours, as
`teleprompt serve --voice` does: no speech model is needed. The app opens
the next script the same way.
`docs/guide/prompter.md#letting-a-voice-read-it` has the rest.

## Drafting from a session

The app has a second mode for a script that doesn't exist yet: **Draft
from a session…** on the welcome page or in the menu, or Ctrl+N. Name the new script, and a
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

A punctuation model gives the draft sentences: `teleprompt setup
punctuation-model --run` installs one the app uses, or choose one in
Settings (see `docs/guide/recording.md`). The microphone is the system's default
input, or ffmpeg's input in `TELEPROMPT_RECORD_MIC`. The server
stops with the app, even if the app is killed. The look is described in
`apps/DESIGN.md`.

## Tests

    cargo test                                             # launching, setup, tools
    TELEPROMPT_BIN=… TELEPROMPT_MODEL=… tests/ui.sh        # and the window
    TELEPROMPT_BIN=… TELEPROMPT_MODEL=… tests/session.sh   # and session mode

The prompter itself is tested as a page, in a browser:
`crates/teleprompt-cli/tests/page`. `tests/ui.sh` runs the app under Xvfb
and checks what it adds: the page shows, a take goes on air with WebKit's
own microphone (`TELEPROMPT_MOCK_MIC`), the screen opens in its own
window, and the server goes when the app is killed. Then it runs the app
again hearing a recording of `apps/fixtures/tour` as its microphone
(`TELEPROMPT_MIC`, a WAV the page hears from when a take starts), and
checks both lines were kept.
