# Teleprompt for Linux

![The prompter mid-take](../../docs/images/prompter.png)

A native prompter for teleprompt, in GTK 4 and libadwaita. Open a script
and read: the text follows your voice, each shot plays as you reach it, and
every line you read in full is kept as that line's take.

The app launches `teleprompt prompt` itself and talks to it over the
prompter API (`docs/design.md#prompter-api-version-1`). It needs:

- `teleprompt` built with the speech recognizer:
  `cargo install --path crates/teleprompt-cli --features listen`
- a speech model: `teleprompt setup speech-model --run` installs
  sherpa-onnx's streaming English model where the app finds it
- GTK 4.14, libadwaita 1.5, VTE for GTK 4 (session mode's terminal),
  GStreamer with the plugins to play the captured clips, and ffmpeg (a
  session's microphone). On Ubuntu 24.04:

      sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev ffmpeg \
        libgstreamer1.0-dev \
        libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-good \
        gstreamer1.0-libav libgtk-4-media-gstreamer

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

## Keys

| key | does |
|---|---|
| Ctrl+Shift+Space | record from the line you are on; during a take, keep it (during the count of three: cancel) |
| click a line | record from there |
| Ctrl+T | record from the top |
| Return | keep the take |
| p | pause or resume |
| r | record the lines reworded since their takes again, one after another |
| e | Edit mode: the shots on the glass, to drag (again: Read mode) |
| w | review a line said in other words than it reads: keep what you said, or the script |
| m | mirror the text, for beam-splitter glass |
| + / − | text size |
| s | show or hide the monitor |
| Ctrl+Shift+S | the monitor in its own window, for a second display |
| Ctrl+Z | undo the last drag on the glass |
| Ctrl+Shift+C | capture the shots not yet captured, or changed since |
| Ctrl+B | capture, then build the video |
| F2 | reword the line you are on, where it stands |
| Ctrl+E | open the script in your own editor |

A take starts after a count of three; Settings turns that off.

## Letting a voice read it

Choose **A voice reads** under "Who narrates" on the welcome page, and the
app reads the script with its voice (`teleprompt prompt --voice`) rather
than following yours. It needs no speech model then: only the binary and
the project's `[voice]`. Play (Space) reads from the line you're on,
lighting the words and playing the shots as it goes; click a line for its
panel (Listen, Read on from here, How to say it, Say it again, Reword);
F2 rewords a line in place. The margin marks each line: a waveform, faint
until the voice has made it; a tick for a line read from your own take.
`docs/guide/prompter.md#letting-a-voice-read-it` has the rest.

## Rewording a line

F2 edits the words of the line you're on where they stand (Enter keeps
them, Escape doesn't), through `teleprompt edit`, so Ctrl+Z undoes it.
Or edit the script while it's open in any editor (Ctrl+E opens it in
yours): the app reloads it, and a
line reworded since its take gets an amber ring in the margin, since its
take no longer says what it says. Press R to record those lines again,
one after another: each is kept as you read past it, and the next starts.

## Keeping what you said

When a take stops, the recognizer hears it again whole, and a line you
said in other words than it reads gets a blue dot beside its tick, and a
toast. Press W (or **Review** on the toast, or **Keep what you said…** in
the menu) to see the difference: the words you didn't say struck through,
the ones you said instead in bold. **Use What I Said** rewords the line
to match with `teleprompt edit <script> said <line>`, and its take stays
current: nothing to record again. **Keep the Script** leaves it, to read
again. A small recognizer mishears, so a word a letter off, or heard as
two ("a round"), doesn't count as other words.

## Editing shots on the glass

The glass is the timeline. Press E, or the pencil in the header, for Edit
mode: each shot shows on the glass itself, as a ribbon under the words it
plays over, or as a pill in the pause after its line when it plays after
it. Drag a ribbon onto a word to start the shot there, into the pause
after a line to play it after, or onto another line to move it; drag its
grip onto a word of its line to end it there. A chip says what a drop will
do before you let go. Each drop is written into the script with
`teleprompt edit`, and refused if the script would no longer compile;
Ctrl+Z undoes the last. Lines don't move: they're as long as the voice
says them. A shot that states no length of its own (a Playwright script, a
composition) moves but doesn't stretch.

Hover a word and the monitor shows, still, what is on screen as it is
said. A click on a line doesn't start a take in Edit mode, and a take
always starts in Read mode, with the shots out of the way.

Where a take was recorded, a word's moment is estimated from its share of
the line's letters, as the markers are, so a ribbon may sit a word off
from where the shot starts in the video.

A moved or stretched shot needs capturing again: Ctrl+Shift+C captures
what's missing, and Ctrl+B captures and builds the video, its progress a
hairline over the tally. When it's built, the toast plays it.

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

A punctuation model gives the draft sentences: `teleprompt setup
punctuation-model --run` installs one the app uses, or choose one in
Settings (see `docs/guide/recording.md`). The microphone is the system's default
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
