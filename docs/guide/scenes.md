# Scenes

An action block names a **scene**: what it shows, and what records it.
The simplest scene is a plugin's own name:

````markdown
```teleprompt scene=vhs
Type "cargo build --release"
Enter
```
````

A scene plugin installed as a [program of its own](plugins.md) is named
the same way: `scene=card` for `teleprompt-scene-card`. `teleprompt
plugins` lists what is installed.

Declare a scene when you want settings for it, or more than one of a
plugin. The name is yours; the plugin does the work:

```toml
[scene.server]
plugin = "vhs"

[scene.client]
plugin = "vhs"
env = { API = "http://localhost:8080" }
```

Blocks of one scene run in one session (see [below](#a-scene-is-a-session)),
so `server` and `client` are two terminals, each carrying on where it left
off. A declared scene named after a plugin, like `[scene.vhs]`, just
gives that plugin settings. A name that is neither declared nor a
plugin is an error.

A path in a scene's settings is relative to the project: the directory
`teleprompt.toml` is in.

| plugin | shows | a shot is | needs |
|---|---|---|---|
| [`vhs`](#terminals) | a terminal | a VHS tape | `vhs` |
| [`asciinema`](#recordings) | a recorded terminal session | part of an asciicast | `agg` |
| [`playwright`](#browsers) | a browser | a Playwright script | Node, Playwright |
| [`remotion`](#motion-graphics) | motion graphics | a composition and its props | Node, a Remotion project |
| [`slidev`](#slides) | slides | a slide at a click step | Node, a Slidev deck |
| [`media`](#images-clips-and-title-cards) | images, clips, title cards | one directive | ffmpeg |
| [`x11`](#desktop-apps) | a Linux app's window | a few actions: keys, typing, the pointer | Xvfb, xdotool, ffmpeg |
| [`macos`](#desktop-apps) | a Mac app's window | the same actions | ffmpeg, and two permissions |

`teleprompt setup` lists what this machine can record with, and `teleprompt
setup <plugin>` says what each needs that is missing, under which
license, and the command that installs it with your own package manager
(`--run` runs it). Teleprompt ships none of these tools.

## Capture

```bash
teleprompt capture scripts/tour.md
```

Capture runs each scene as **one session** and keeps a clip for every shot
that doesn't have one. `build` captures on the way past, so you only need
this command to fill the cache before a render, or after editing a tape.

A shot whose clip is already cached **still runs**. Its clip isn't needed,
but the screen it leaves behind is, because the next shot opens on it. A
session with nothing left to capture isn't opened, and a session stops
after the last shot it needs.

A scene this machine can't record produces a warning and a slate, not a
failed build. The timing is still real, and a video with a gap in it is
more use than no video.

### A scene is a session

The steps of a walkthrough build on one another: a running program, a
selected row, an open log. So blocks naming the same scene run in the same
session, and the screen one block leaves is the screen the next starts
from. Six narrated steps are six blocks sharing one terminal, not six
fresh shells.

It follows that a clip isn't named by its own source. `Type "j"` might
appear twice in a walkthrough, once to move down a list of pipelines and
once to move down a list of tasks. That's one tape and two pictures. Each
shot therefore has a `capture_key`: the hash of its own source plus every
source before it in the session. Editing a block re-captures that block
and everything after it in its session, and nothing else. Moving a
paragraph changes start times but no keys.

`session="…"` on a block starts a different run of the scene, for a
script that quits a program and starts it again:

````markdown
```teleprompt scene=vhs session=retry
Type "flowrs"
Enter
```
````

The session's name isn't part of the key. A run that opens with the same
tape opens on the same screen, so it gets the same clip.

**A capture runs the commands.** There's no sandbox and no dry run. A tape
that types `teleprompt new demo` scaffolds a project, and one that types
`rm` removes something. That's what makes the video true. The shell starts
in the directory the build was run from.

## Terminals

A `vhs` block is a [VHS](https://github.com/charmbracelet/vhs) tape,
and `vhs` records it:

````markdown
```teleprompt scene=vhs
Set TypingSpeed 35ms
Type "cargo build --release"
Enter
Sleep 2s
# mark
Type "./target/release/acme --help"
Enter
Sleep 1s
```
````

`# mark` ends a shot, which is where the next paragraph of narration can
start. It's a comment on purpose: VHS ignores it, so the block stays a
tape `vhs` itself will run. You can also point the fence's `include=` at a
real `.tape` file, and at one part of it with `include=file.tape#2`: `#2`
is the part after the first `# mark`, `#2-3` a range.

**A tape states its own timing, so its shots are timed exactly.** Every
`Sleep` is written down and every keystroke costs `Set TypingSpeed` (50 ms
by default, as in VHS), so teleprompt adds them up rather than guessing.
The exception is `Wait`, which blocks until a program finishes. A shot
containing one takes its timeout (`Set WaitTimeout`, 15 s by default) and
is reported as `estimated`.

**The tape `vhs` runs is the schedule.** teleprompt re-times each shot to
last as long as the sentence over it (see [policies](scripts.md#pairing-and-policies)),
concatenates a session's shots, and runs `vhs` once per session. The shots
are then windows onto that one recording.

`Hide` stops the recording and `Show` resumes it, which is how a tape does
its setup off camera. The commands still run, but hidden time costs the
narration nothing. teleprompt uses the same mechanism to hide the shell's
startup.

`check` refuses a few commands, each with a reason:

- `Output` and `Screenshot`, because teleprompt owns what a capture writes.
- `Set Shell` and `Env`, because the terminal belongs to the scene, set up
  before its first block runs.
- `Source`, because its marks would be invisible until run time. Use
  `include=` instead.
- `Set PlaybackSpeed`, because re-timing the finished recording would slide
  the narration out from under it. `policy=fit-action` is the way to
  change a shot's pace.
- A setting it doesn't recognise. `Set TypingSped 10ms` is a typo, and
  passing it through would type at a speed you didn't choose while `check`
  reported success.

Settings persist across a `# mark` but not across blocks, so a block that
wants a non-default typing speed sets it itself.

```toml
[scene.vhs]
# settle_ms = 800       # hidden time for the shell to start
# font_size = 22
# theme = "Dracula"

[scene.vhs.env]
PATH = "target/release:/usr/local/bin:/usr/bin:/bin"
```

The frame comes from `output.resolution`. Everything in the scene's
configuration is part of the capture key, so changing the terminal
re-records it.

**A scene's `PATH` extends yours rather than replacing it.** VHS finds
`ttyd` and its headless Chromium through `PATH`, so the directories you
name come first and the `PATH` teleprompt was run with follows them.

**Pin `vhs` to v0.11.0.** v0.12.0 runs every command of a tape, exits 0 and
writes no file
([#787](https://github.com/charmbracelet/vhs/issues/787)). teleprompt
checks for the output rather than trusting the exit code, so this shows up
as a capture error naming the missing file.

## Recordings

Some sessions shouldn't run again at capture time: a deploy, a migration,
a build that takes twenty minutes. The `asciinema` plugin plays back a
cast that [`asciinema rec`](https://asciinema.org) already made:

````markdown
First, it checks a script, which costs nothing and changes nothing. {#check}

```teleprompt scene=asciinema policy=fit-action include=casts/tour.cast#check
```

Then it plans the script. {#plan}

```teleprompt scene=asciinema policy=fit-action include=casts/tour.cast#plan
```
````

The block is the cast, v2 or v3, and **its own markers split it**. These
are the `[12.5, "m", "plan"]` events `asciinema play` pauses at.
`include=file#fragment` selects part of the cast: `#2` is the part after
the first marker, `#2-3` a range, and `#plan` the part that a marker
labelled `plan` begins. Blocks of one scene are one session, and the
terminal carries on from one to the next.

A cast states its timing in full, so every shot is `exact`, with the
recording's `idle_time_limit` applied the way `asciinema play` applies it.
`policy=fit-action` re-times a shot by moving its pauses, never its
keystrokes. A line too short for even the shortest pauses plays the
whole session faster, so the end is never cut off. `check` reads the cast and reports a bad line at its line
number in the cast file.

```toml
[scene.asciinema]
theme = "dracula"   # any agg theme
font_size = 32
```

Capture lays the session's shots end to end in one cast, renders it with
[`agg`](https://github.com/asciinema/agg), and cuts it into a clip per
shot. See `examples/asciinema`.

## Browsers

A `playwright` block is a Playwright script, run as written, with `page`
in scope:

````markdown
```teleprompt scene=playwright
await page.goto('http://localhost:3000');
await page.getByText('Get started').click();
// mark
await page.getByLabel('Name').fill('demo');
```
````

`// mark` splits it into shots, and is again a comment so the block stays
a script you could paste into a test. A script doesn't state its own
timing, since `page.click()` takes as long as the page takes. So each shot
takes the length of its sentence, and if the script finishes early the
page holds still for the rest of the slot.

A block can instead run one test of a test file you already have,
named by its title. Its body is run as the shots, without the test's hooks
or fixtures beyond `page`:

````markdown
```teleprompt scene=playwright include="e2e/checkout.spec.ts#pays by card"
```
````

In a plain script, `include=script.js#2` is the part after its first
`// mark`, as for a tape.

```toml
[scene.playwright]
# executable = "/path/to/chromium"
# cursor = "pointer"           # the pointer drawn over clicks
# labels = "bottom-right"      # where action labels appear
# annotation_ms = 1200         # how long each action is shown
# annotation_size = 32
```

## Motion graphics

The `remotion` plugin renders compositions from an existing
[Remotion](https://www.remotion.dev) project, unchanged. A shot names a
composition and its props, as `npx remotion render <id> --props=…` takes
them:

````markdown
The words decide the timing. {#timing}

```teleprompt scene=remotion policy=concurrent
Pipeline {"steps": ["Markdown", "Kokoro", "Timeline", "Video"]}
```
````

Props are a JSON object and may span lines, and `#` lines are comments.
`check` refuses props that aren't JSON. A composition the project doesn't
register fails at capture, with Remotion's own error. A shot lasts as long
as the paragraph above it, so each composition gets a paragraph and block
of its own.

```toml
[scene.remotion]
project = "motion"          # the Remotion project, with node_modules installed
# entry = "src/index.ts"    # its registerRoot file; Remotion's default
# browser = "/path/to/chrome-headless-shell"
```

**teleprompt owns the length and the frame, and the composition owns the
rest.** Each shot renders with `durationInFrames` set to its sentence and
with the script's resolution and fps. A component that animates against
`useVideoConfig().durationInFrames` fills a two-second slot and a
fifteen-second one alike.

A composition draws the same frames whatever came before it, so shots
don't chain: editing one paragraph re-renders one shot. The capture key
also covers what the bundler reads (the entry's directory, `public/`,
`package.json`, `package-lock.json` and `remotion.config.ts`), so editing
a component re-renders the scene. It doesn't follow the import graph, so
changing one component re-renders shots that never used it.

Remotion downloads a headless Chrome on first render unless `browser` or
`TELEPROMPT_REMOTION_BROWSER` names one. See `examples/remotion`:

```bash
(cd examples/remotion/motion && npm install)
cargo run -- build examples/remotion/scripts/remotion.md
```

## Slides

The `slidev` plugin shows slides from an existing
[Slidev](https://sli.dev) deck. A shot names a slide the way Slidev's URLs
do. `3` is slide three as it opens, and `3?clicks=2` is slide three after
two of its `v-click`s, so a list can be revealed a sentence at a time:

````markdown
You can still present it with Slidev. {#present}

```teleprompt scene=slidev policy=concurrent
2?clicks=1
```
````

```toml
[scene.slidev]
deck = "talk/slides.md"     # Slidev is found in a node_modules beside or above it
# dark = true
# browser = "/path/to/chrome"
```

Capture runs one `slidev export --with-clicks` per session, of just the
slides it needs. **A still is the same picture at any length**, so the
length isn't in the key and rewording a sentence reuses its slide. The key
does cover the deck and what Slidev reads beside it (`components/`,
`layouts/`, `public/`, `styles/`, `setup/`, `pages/`, `snippets/` and the
package manifests).

A still can't show motion: Slidev's own transitions and `v-motion`
animations aren't in the picture, and the cut between blocks is
teleprompt's transition. A slide the deck doesn't have fails at capture,
naming the slide and how many clicks it does have. Slidev's Playwright
drives the export unless `browser` or `TELEPROMPT_SLIDEV_BROWSER` names a
browser. See `examples/slidev`, whose script was
[drafted from its speaker notes](scripts.md#from-a-slidev-deck).

## Desktop apps

A desktop app is shown by running it, as a terminal is: the `x11` plugin
on Linux and `macos` on a Mac. A scene names the app, and its blocks are
what someone at the keyboard does:

```toml
[scene.app]
plugin = "x11"
command = "gnome-text-editor notes.md"
```

````markdown
Open the command palette and search for the setting. {#palette}

```teleprompt scene=app policy=fit-action
Ctrl+Shift+P
Sleep 500ms
Type "tab width"
Sleep 1s
Enter
```
````

| line | does | takes |
|---|---|---|
| `Type "…"`, `Type@80ms "…"` | types the text | a `TypingSpeed` per character |
| `Enter`, `Escape`, `Ctrl+O`, `Down 3` | presses a named key, or a chord | a `TypingSpeed` per press |
| `Key e`, `Key +`, `Key Ctrl++` | presses a single character | as above |
| `Move 640 360`, `Click …`, `DoubleClick …`, `RightClick …` | glides the pointer to a point in the window, from its top-left corner, and clicks there | a `PointerSpeed` (400 ms) |
| `Wait "saved"`, `Wait@5s "…"` | until the window's title contains the text | up to `WaitTimeout` (15 s) |
| `Sleep 2s` | nothing, while the app goes on | as written |
| `Set TypingSpeed 60ms`, `Set PointerSpeed …`, `Set WaitTimeout …` | changes a speed for the rest of the block | nothing |
| `# mark` | ends a shot | |

It is VHS's vocabulary with a pointer: a lone letter is written `Key e`,
since a line reading `E` is likelier a mistake, and `Cmd+` and `Super+` are
the same modifier. A block without a `Wait` states its length exactly, so
`fit-action` can stretch it to its line by scaling its pauses and speeds.
With a `Wait` its length is a bound.

Blocks of a scene run in one session of the app, one after another, and
each shot is held to its slot while the app carries on. So a take started
in one block is still running in the next, as it would be on the screen.
The app's own time can't be predicted, so a shot is cut from the recording
where it really began rather than where the schedule said it would.

```toml
[scene.app]
plugin = "x11"
command = "my-app --demo"   # run by sh, in the project's directory
title = "My App"            # the window to follow, if it has several
ready = "demo.db — My App"  # wait, unrecorded, until the title says this
settle_ms = 1500            # then let it draw
launch_timeout_ms = 30000
env = { LANG = "en_US.UTF-8" }
```

The app's loading isn't recorded: the recording starts once its window is
up, fitted to the frame, and has the `ready` title if the scene sets one.

**On Linux**, each session gets its own virtual display (`Xvfb`) at the
video's size, so a capture shows the app and nothing else, and runs the
same on a laptop or a server. The app runs in its own D-Bus session
(`dbus = false` turns that off), with `GDK_BACKEND=x11` and
`QT_QPA_PLATFORM=xcb`, so GTK and Qt apps draw on the virtual display even
under Wayland. `xdotool` presses the keys and moves the pointer.

**On a Mac** there is no virtual display, so the app runs on your screen:
brought to the front, its window fitted to the frame, and the recording
cropped to it. Don't use the Mac while it records, since keys go to
whatever is in front. Whatever runs `teleprompt` (your terminal) needs two
permissions in System Settings → Privacy & Security: Accessibility, to
press keys and move the pointer, and Screen Recording, for ffmpeg to see
the screen. `command` can be a binary or `open -W -n -a "App"`; with
`open`, set `process = "App"` to name the process whose window to follow.
`screen` picks a display by ffmpeg's name for it (`Capture screen 0`).

See `examples/desktop`, a tour of teleprompt's own Linux app.

## Images, clips and title cards

The `media` plugin needs only ffmpeg. One directive per shot:

````markdown
```teleprompt scene=media
image src=arch.png fit=contain
```
````

| directive | draws | length |
|---|---|---|
| `image src=… fit=contain\|cover` | a still, letterboxed or cropped to fill | its sentence's |
| `clip src=… from=0:12 to=0:19 fit=…` | that range, without its sound | exact, `to - from`; without `to`, its sentence's |
| `title text="…" subtitle="…"` | text on a colour | its sentence's |

A clip that ends before its sentence holds its last frame. A clip is never
re-timed, since that would change its speed. Times are written `0:12`,
`12.5s` or `250ms`, and `check` suggests the nearest spelling of a
mistyped key. A title's text is drawn as written, quotes, colons and `%`
included.

```toml
[scene.media]
dir = "media"            # what `src` is relative to, from the project
background = "#0b0d10"   # behind letterboxing, and a title's colour
color = "#eef3f8"        # a title's text
# font = "Sans"
```

Each shot's key covers the file it shows, so replacing an image re-draws
only the shots that show it. See `examples/media`.
