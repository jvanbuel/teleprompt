# Teleprompt for macOS

Teleprompt's prompter in a window of its own. Open a script and read: the
text follows your voice, each shot plays as you reach it, and every line
you read in full is kept as that line's take.

The prompter is the page `teleprompt serve` serves, the same page a
browser shows, and so is setting teleprompt up. The app opens on a
welcome page of its own: open a script or one of those opened last (also
under File › Open Recent), and who narrates. It launches the server
itself in the script's project, shows its page in WebKit, gives it the
microphone (macOS asks you first), and opens its screen in a second
window when it asks. Around it the app has settings. What the prompter does, and its keys, are in
`docs/guide/prompter.md`; `?` lists the keys in the app.

It needs:

- `teleprompt` built with the speech recognizer:
  `cargo install --path crates/teleprompt-cli --features listen`
- a speech model: `teleprompt setup speech-model --run` installs
  sherpa-onnx's streaming English model where teleprompt finds it, or the
  page offers to when you open a script

Set the binary in Settings (⌘,) the first time if it isn't where
`cargo install` or Homebrew puts it. To have the script's voice read it
instead, choose **A voice reads** under "Who narrates" on the welcome page:
only the binary is needed then, built with or without the recognizer.

## Build

    ./bundle.sh            # Teleprompt.app, here
    open Teleprompt.app

`swift run Teleprompt` works too, but then macOS asks for the microphone on
behalf of your terminal.

## The app's keys

| key | does |
|---|---|
| ⌘O | open a script |
| ⇧⌘O | the welcome page, with the scripts opened last |
| ⌘E | open the script in your own editor |
| ⌘, | settings |

The prompter's keys are the page's, the same as in a browser. A take
starts after a count of three; Settings turns that off. The look is
described in `apps/DESIGN.md`; the typeface and the icon are bundled by
`bundle.sh`.

## Layout

- `Sources/TelepromptKit`: launching the server. Foundation only; builds
  and is tested on Linux too.
- `Sources/Teleprompt`: the SwiftUI app, and the page in WebKit
  (`PrompterPage.swift`).
- `Tests/TelepromptKitTests`: launching, against
  `docs/api/v1/examples/listening.json`.

The prompter itself is tested as a page, in a browser:
`crates/teleprompt-cli/tests/page`.
