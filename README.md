# teleprompt

Compile narrated videos from version-controlled Markdown.

A script's prose is its narration; fenced `teleprompt` blocks are its visuals.
Narration duration drives visual pacing, so editing a paragraph changes the
rhythm of the video — and `teleprompt diff` tells you exactly how before you
render anything.

The compiler and the feedback loop work end to end: parsing, narration
timing, timeline scheduling, a live preview, and `build`, which renders a
video with ffmpeg. Terminal scenes are written as
[VHS](https://github.com/charmbracelet/vhs) tapes and compile to exact
durations.

**Nothing captures scenes yet.** teleprompt reads a tape; it does not run
one. A render therefore holds each beat's slot with a slate — the timing is
the scheduled timing, and the picture is not there yet. `build` says how
many, every time. See
`docs/superpowers/specs/2026-08-15-teleprompt-design.md` for the design.

## Try it

```bash
cargo run -- new demo
cargo run -- plan demo/scripts/demo.md
cargo run -- diff demo/scripts/demo.md
```

`new` scaffolds a project with a demo script. `plan` compiles it into a
timeline and prints one line per beat, with its narration duration and
policy. `diff` compares that timeline against whatever is committed — on a
fresh project there's nothing committed yet, so every beat shows up as
`added`.

Now edit a paragraph in `demo/scripts/demo.md`, run `diff` again, and watch
the transitions move — that's the whole feedback loop. `plan`'s JSON output
is what actually gets committed to `demo/timelines/`, so committing it after
`plan` is what makes the next `diff` compare against something real:

```bash
cargo run -- plan demo/scripts/demo.md --format json > demo/timelines/demo.en.json
# edit demo/scripts/demo.md
cargo run -- diff demo/scripts/demo.md
```

## Commands

| command | does |
|---|---|
| `new <path>` | scaffold a project |
| `from <doc>` | draft a script from a Markdown document you already have |
| `check <script>` | parse and validate; no side effects, no cost |
| `plan <script>` | compile the timeline and print it |
| `diff <script>` | compare against the committed timeline |
| `dub <script> --out <dir>` | synthesize narration and write audio plus a manifest |
| `serve <script>` | live preview that opens on the beat you just changed |
| `build <script>` | render the video |
| `capture` | record the scenes a build will show |
| `doctor` | report the environment teleprompt can see |
| `cache` | report what the project's caches hold, or shrink them |

`doctor` also lists the scene adapters and voice backends this build ships.

All commands accept `--format json`.

### Estimated versus measured durations

`plan`, `check`, and `diff` never synthesize, never read audio, and never
await. They read durations from the cache — metadata only, never the WAV
bytes — and fall back to a word-count estimate for anything not yet rendered,
so the inner loop stays instant and offline no matter how slow the configured
voice is. Only `dub` starts an async runtime.

```
$ teleprompt plan scripts/tour.md --format json | grep duration_source
  "duration_source": "estimated"
```

`teleprompt dub` does the real synthesis and fills the cache. Afterwards the
same `plan` is still instant, and now says `measured`.

A timeline committed from a cold cache will drift the first time you `dub`,
which `diff` reports as `now measured` rather than as a content edit. `plan`
warns when it emits estimates for exactly that reason.

## Starting from a document you already have

```bash
teleprompt from README.md
```

Prose becomes narration segments with their ids promoted at birth, so the
first edit cannot shift them (§3.3). Shell code blocks become terminal tapes
that **type** the command and never run it — marked `review=pending`, which
`check` warns about on every run until a human has read the tape and removed
the attribute. Everything else — a JSON payload, a TypeScript snippet — stays
ordinary Markdown, which compilation ignores, so nothing is invented and
nothing is lost.

The result is a draft, not a script: no pacing is inferred, every block gets
the default policy, and choosing `concurrent` over `hold` is exactly the
judgement you are there to make.

### From a Slidev deck

```bash
teleprompt from talk/slides.md --slidev --out scripts/talk.md
```

A deck's speaker notes are already the thing a script is: what is said
over each slide. `--slidev` reads them the way Slidev does — a slide's
notes are its last HTML comment — and turns each into a paragraph followed
by a block showing that slide. Slidev's own `[click]` markers split a note
into one paragraph per click step, so a list revealed on click is
narrated a point at a time:

```md
<!--
The deck does not change to be narrated.
[click] You can still present it with Slidev.
[click] You can still export it.
-->
```

becomes three paragraphs over `2`, `2?clicks=1` and `2?clicks=2`.
`[click:3]` adds three clicks, as it does in Slidev. The draft names the
deck in its own front matter, relative to where you ran `from`, which is
where the build will run. A slide with no notes has nothing to be said
over it, so it is left out and `from` names it.

As with any draft, the script is the source of truth from here on: `from`
will not overwrite it, and the notes and the script are free to part ways.

## Watching what an edit costs

`plan` and `diff` answer in milliseconds, and they answer in numbers. Judging
an edit means hearing it:

```bash
teleprompt serve scripts/tour.md
```

Save the script and the preview recompiles, synthesizes only the segments
whose text changed, and opens on the first beat that moved — with the ones
that shifted marked on the timeline strip. A paragraph rewritten in an editor
is audible in the browser about a second later.

**The preview is a manifest consumer.** It reads exactly what
`docs/integrations/remotion.md` tells an outside integrator to read — the
published `narration.json`, with its `beats` — so the thing you watch while
writing cannot drift from what a renderer produces. The only extra it asks
teleprompt for is each span's source, which it needs to draw a terminal and
which a consumer drawing its own picture would not.

A script that stops compiling does not blank the preview: the error appears
and the last version that compiled keeps playing.

## Saying it when you say it

A concurrent action starts with its paragraph, which is right when the
paragraph is about the action from its first word and wrong the moment the
sentence naming the command is the third one. `at=` anchors it:

```markdown
One command registers a server. flowrs config add asks for a name. {#config}

```teleprompt scene=terminal policy=concurrent cue="flowrs config add"
Type "flowrs config add"
Enter
```
```

The typing starts when the voice reaches that phrase. A cue that names
something the paragraph does not say is an error, not a silent no-op, and a
cue only means something under `policy=concurrent` — `hold` puts the action
after the narration and the stretch policies size it to fit.

Where a backend publishes word timings the offset is exact. Where none does
— which is every backend today — it is interpolated from where the phrase
sits in the sentence, which lands within a syllable or two and is the
difference between typing a command while it is named and typing it half a
paragraph early.

## Saying it the way it is said

A synthetic voice reads spelling. `MWAA` comes out as a word; so does every
acronym and half the product names:

```yaml
voice:
  pronounce:
    MWAA: em-double-you-ay-ay
    TUI: tee-you-eye
```

Applied to synthesis only — the script, the captions and the manifest keep
the spelling, because that is what a reader wants to see. Whole words,
case-sensitively, and the mapped text is in the cache key, so correcting a
word re-renders exactly the audio that said it wrong.

## Capture

```bash
teleprompt capture scripts/tour.md
```

Stage 5, between the manifest and the render. It runs each scene as **one
session** and keeps a clip for every beat that has none, filed under the
beat's capture key. `build` does this on the way past, so the command is
for filling a cache before a render or after editing a tape.

The unit is the session rather than the beat, and that has a consequence
worth stating: a beat whose clip is already cached **still runs**. Its clip
is not needed; the screen it leaves behind is, because the beat after it
opens on it. A session with nothing left to keep is not opened at all, and
a session stops after the last beat worth keeping — on a tape whose sleeps
are real seconds, that is the difference between a capture that stops and
one that sits there.

A scene this build cannot record is a warning and a slate, not a failed
build: the timing is still real, and a video with a hole in it is more use
than no video. `teleprompt doctor` lists what can record what.

### Terminals

`terminal` scenes are recorded by [VHS](https://github.com/charmbracelet/vhs)
— see **Terminal scenes** below for how the tape gets to it.

The terminal is the scene's, not the tape's — the same reason `Set Shell`
is refused at compile time:

```toml
[scene.terminal]
adapter = "vhs"

[scene.terminal.env]
PATH = "target/release:/usr/local/bin:/usr/bin:/bin"
```

`settle_ms`, `font_size`, `theme` and anything under `env` are read here;
the frame comes from `output.resolution`, and the tape says only what to
type. All of it is in the capture key, so changing the terminal re-records
the scenes it changed.

**A scene's `PATH` extends, it does not replace.** VHS applies `Env` to
its own process rather than only to the shell it records, and it finds
`ttyd` and the headless Chromium it draws through by searching `PATH`. A
scene that replaced `PATH` outright would disarm the recorder — and say
so in the wrong vocabulary, complaining that `ttyd` is missing or that it
could not get a debug URL, about a setting you wrote to make your own
binary findable. So the directories you name here come first and win, and
the `PATH` teleprompt was run with follows them as a fallback.

**A capture runs the commands.** There is no sandbox and no dry run: a
tape that types `teleprompt new demo` scaffolds a project, and one that
types `rm` removes something. That is what makes the video true, and it is
why `check` refuses `Output` and `Set Shell` — teleprompt owns the
terminal a tape runs in, and an author owns what the tape does in it. The
shell starts in the directory the build was run from unless the scene says
`cwd`.

### Motion graphics

A scene whose adapter is `remotion` renders compositions from an ordinary
[Remotion](https://www.remotion.dev) project — the one you already have,
unchanged. A shot names a composition and its props, which is what
`npx remotion render <id> --props=…` takes:

````markdown
The words decide the timing. {#timing}

```teleprompt scene=motion policy=concurrent
Pipeline {"steps": ["Markdown", "Kokoro", "Timeline", "Video"]}
```
````

Props are a JSON object and may span lines, and `#` lines are comments.
`check` refuses props that are not JSON; a composition the project does
not register fails at capture, with Remotion's own error. A shot lasts as
long as the paragraph above it, so each composition gets a paragraph and
a block of its own: a shot after a `# mark` has no sentence to take its
length from, and `check` refuses it rather than let it last no time.

```toml
[scene.motion]
adapter = "remotion"
project = "motion"          # the Remotion project, with node_modules installed
# entry = "src/index.ts"    # its registerRoot file; Remotion's default
# browser = "/path/to/chrome-headless-shell"
```

**teleprompt owns the length and the frame; the composition owns the
rest.** Each shot is rendered with `durationInFrames` set to the sentence
spoken over it and the script's resolution and fps, whatever the
composition registered. A component that animates against
`useVideoConfig().durationInFrames` fills a two-second slot and a
fifteen-second one alike. The length is written into the published source,
so a reworded sentence re-renders the shot beneath it.

**A shot does not continue the one before it.** A terminal shot opens on
the screen its predecessor left, so its key chains through every shot
before it. A composition draws the same frames whatever preceded it, so
each shot is named by itself: editing one paragraph re-renders one shot.

One Node process bundles the project's entry once and renders each missing
shot to its clip, using the project's own `node_modules`. Remotion
downloads a headless Chrome on first render unless `browser` or
`TELEPROMPT_REMOTION_BROWSER` names one.

The key also covers what the bundler reads: the entry point's directory,
`public/`, `package.json`, `package-lock.json` and `remotion.config.ts`,
hashed by content (`node_modules` and dot-files aside). Editing a component
re-renders the scene's shots; reverting the edit finds the old clips again.
The granularity is the scene, not the composition — teleprompt does not
read the bundle's import graph, so changing one component re-renders shots
that never used it.

`examples/remotion` is a complete project — a script narrated by Kokoro
whose every picture is a composition:

```bash
(cd examples/remotion/motion && npm install)
cargo run -- build examples/remotion/scripts/remotion.md
```

### Slides

A scene whose adapter is `slidev` shows slides from an ordinary
[Slidev](https://sli.dev) deck. A shot names a slide the way Slidev's own
URLs do: `3` is slide three as it opens, `3?clicks=2` is slide three after
two of its `v-click`s. So a list can be revealed a sentence at a time, one
paragraph and one block per click:

````markdown
You can still present it with Slidev. {#present}

```teleprompt scene=slides policy=concurrent
2?clicks=1
```

You can still export it. {#export}

```teleprompt scene=slides policy=concurrent
2?clicks=2
```
````

```toml
[scene.slides]
adapter = "slidev"
deck = "talk/slides.md"     # the Slidev entry; Slidev is found in a node_modules beside or above it
# dark = true               # export with the dark theme
# browser = "/path/to/chrome"
```

Capture is one `slidev export --with-clicks` per session, of just the
slides it needs, and each still becomes a clip the renderer holds for the
sentence. **A still is the same picture at any length**, so the length is
not in the key: rewording a sentence reuses the slide it was spoken over.
The key does cover the deck and what Slidev reads beside it —
`components/`, `layouts/`, `public/`, `styles/`, `setup/`, `pages/`,
`snippets/` and the package manifests — so editing the deck re-exports.

What a still cannot show is motion: Slidev's own slide transitions and
`v-motion` animations are not in the picture, and the cut between two
blocks is teleprompt's transition. `check` refuses a shot that is not a
slide; one the deck does not have fails at capture, naming the slide and
how many clicks it does have. Slidev drives a browser to export, found by
its own Playwright unless `browser` or `TELEPROMPT_SLIDEV_BROWSER` names
one.

`examples/slidev` is a complete project, whose script was drafted from
its deck's speaker notes (see **From a Slidev deck** above):

```bash
(cd examples/slidev/deck && npm install)
cargo run -- build examples/slidev/scripts/slides.md
```

## Rendering

```bash
teleprompt build scripts/tour.md
```

Synthesizes narration, publishes the manifest, and renders
`build/tour.en.mp4` — narration placed at the offsets the manifest
published, each beat holding its scheduled slot, transitions overlapped the
way the scheduler granted them. `--resolution` and `--fps` override the
script's own `output:` block for one render; `teleprompt doctor` reports
whether this machine has an ffmpeg to do it with.

`build` reads the published manifest rather than the timeline it just
compiled, exactly as an outside integrator does. Two timing paths drift, and
the one that drifts silently is the one nobody renders from.

A build encodes the picture in pieces and keeps them. Each piece is keyed
on everything that changes its frames — the clip's contents, its length,
the frame size and rate, the encoder's own settings — and a rebuild copies
the pieces that still match rather than encoding them again. Rewording one
sentence of the manual re-encodes the picture around that sentence and
copies the other five and a half minutes: 18.4s becomes 4.2s. A piece is
keyed on its own duration rather than on where it falls, so a sentence that
got longer moves everything after it without invalidating any of it.
`--no-cache` encodes every frame.

The cache has a cap, because a cache of video does not stay small: every
version of a script you iterate on leaves its picture behind. A build
evicts the least recently used pieces until what is left fits in 1 GB —
`--cache-max-mb` for a different budget, `0` to keep nothing. `teleprompt
cache` reports what is there and `teleprompt cache --prune-to-mb N` shrinks
it now, so getting the disk back does not mean being told which directory
to delete. Everything in it is derived, so evicting costs time and nothing
else.

Rendering sits behind a trait. ffmpeg handles everything, transitions
included; a pure-Rust path for scripts whose beats are all hard cuts —
concat, mix, mux — is what would make a single static binary possible for
those, and is not written yet.

## Dubbing a pipeline that renders itself

`teleprompt dub` writes narration audio and a manifest, and renders no video.
A tool teleprompt does not control — Remotion, After Effects, a web player —
reads the manifest and owns its own picture.

```bash
teleprompt dub scripts/tour.md --out public/narration
```

```
public/narration/en/narration.json
public/narration/en/audio/welcome.wav
```

The manifest gives every segment an id, its prose, an absolute `start_ms` and
`duration_ms`, the audio path, and which voice tier actually produced it.
Deriving your composition's length from `duration_ms` is what keeps
teleprompt's central property working on the other side of the boundary:
narration length still drives pacing.

### Consuming it from Remotion

```tsx
const msToFrames = (ms: number, fps: number) => Math.round((ms * fps) / 1000);

calculateMetadata={async ({props}) => {
  const m = await (await fetch(staticFile(props.manifestPath))).json();
  if (m.manifest_version !== 2) throw new Error(`unsupported manifest ${m.manifest_version}`);
  return {durationInFrames: msToFrames(m.duration_ms, 30), props: {...props, manifest: m}};
}}
```

A segment's length is its **own** `start_ms + duration_ms`, never the next
segment's `start_ms`. Consecutive segments may overlap — the transition
window is subtracted from the preceding beat — so inferring a length from
the next start clips the tail of the speech:

```tsx
const from  = msToFrames(seg.start_ms, fps);
const until = msToFrames(seg.start_ms + seg.duration_ms, fps);
// durationInFrames={until - from}
```

Convert **absolute offsets** to frames and take the difference, as above —
never round a duration on its own, or the rounding error accumulates and
drifts audio out of sync by the end of a long video.

### Keeping it honest

```bash
teleprompt dub scripts/tour.md --out public/narration --check
```

Exits `3` when the committed manifest no longer matches the script, so a pull
request that edits prose without re-dubbing fails CI. Commit
`narration.json`; `audio/` is reproducible and can be gitignored, at the cost
of needing a voice backend wherever you render.

`--check` writes nothing to `--out`, but it does synthesize anything not
already cached and populate `.teleprompt/cache` with it — it has to, or it
would be comparing against numbers it had not measured. So it needs a
writable checkout on a cold cache, and a first `--check` in CI costs a full
render.

`docs/integrations/remotion.md` has the whole integration: a complete example
project, what every manifest field is for, and the two ways to get the frame
arithmetic wrong.

The manifest route takes no Remotion dependency; the `remotion` scene
adapter (see **Motion graphics**) is the other direction, where teleprompt
owns the video and Remotion draws one scene of it. Remotion's
own licence — free for individuals and organisations up to three employees —
is between you and Remotion.

## Terminal scenes

An action block whose `scene=terminal` is a [VHS](https://github.com/charmbracelet/vhs)
tape — Charm's recording language, which teleprompt reads and runs itself:

```teleprompt scene=terminal
Set TypingSpeed 35ms
Type "cargo build --release"
Enter
Sleep 2s
# mark
Type "./target/release/acme --help"
Enter
Sleep 1s
```

`# mark` ends a span, which is where the next paragraph of narration gets to
start. It is spelled as a comment on purpose: VHS ignores it, so the body stays
a tape `vhs` itself will run — which is the whole reason to point a fence's
`include=` at a real `.tape` file rather than at a dialect only teleprompt
reads.

**`vhs` renders it.** teleprompt already re-times every tape so a span
lasts exactly as long as the sentence over it (see `policy=` below), so
the tape handed to `vhs` *is* the schedule: run it, and the video's
timeline is the timeline. One run per session — the spans are
concatenated in order so the program stays running across beats — and the
beats are then windows onto the one video, at the offsets the tape was
written to produce. Nothing is estimated and nothing is reimplemented:
`vhs` was built to run tapes, and the only thing teleprompt has to know is
how to write one.

The tape language is still read twice, by the compiler that times it and
by `vhs` that runs it — so `check` refuses two commands rather than let
them mean different things in the two places. `Env` belongs to the scene,
because a scene is a session whose shell is settled before its first block
runs; `Screenshot` belongs to teleprompt, because it owns what a capture
writes. Both are refused for the reasons `Set Shell` and `Output` are.

`Hide` stops the recording and `Show` resumes it, which is how a tape does
its setup without every demo opening on somebody navigating to a
directory. The commands still run. Their time is not the beat's — a beat
lasts as long as something is on screen, and nothing hidden is — so hidden
work costs the narration nothing. It is also how teleprompt hides the
shell's own startup: the tape it writes opens with a hidden settle, so no
beat begins on a prompt being drawn.

**Pin your `vhs`.** v0.12.0 runs every command of a tape, prints
`Creating <file>.gif…`, exits 0 and writes no file — including from its
own `vhs new` example ([#787](https://github.com/charmbracelet/vhs/issues/787),
open at time of writing; v0.11.0 records the same tape). teleprompt checks
for the output rather than trusting the exit code, so this surfaces as a
capture error naming the file that never appeared instead of a video of
slates.

**A tape states its own timing, so its spans are timed exactly.** Every `Sleep`
is written down and every keystroke costs `Set TypingSpeed` (50 ms by default,
matching VHS), so teleprompt adds them up instead of guessing, and never has to
run a terminal to find out how long the terminal takes:

```
$ teleprompt plan manual/scripts/cli.md --format json | grep -A4 '"adapter": "vhs"'
        "adapter": "vhs",
        "span_hash": "c590e04a8aa99440733a0ad195e0f6c4e281e62c2e82bd16a9880fd52fb83ad4",
        "start_ms": 20650,
        "duration_ms": 2630,
        "duration_source": "exact"
```

The one exception is `Wait`, which blocks on a program rather than on a clock.
A span containing one contributes that `Wait`'s timeout (`Set WaitTimeout`,
5 s by default) and is reported as `estimated`; every other span is `exact`.

A few commands are refused on purpose, each with the reason: `Output` and
`Set Shell` (teleprompt owns the capture and the terminal), `Source` (use the
fence's `include=` instead, so the marks are visible before run time), and
`Set PlaybackSpeed` — re-timing the finished recording would slide the
narration out from under the action it was scheduled against, which is what
`policy=stretch-action` is for. So is a setting teleprompt does not recognise:
`Set TypingSped 10ms` is a typo, and passing it through would mean a tape that
types at a speed its author did not choose while `check` reports success.

Settings persist across a `Mark` but not across a fence: each action block is
validated and split on its own, so a block that wants a non-default typing
speed says so itself.

## A scene is a session

The beats of a walkthrough continue one another — a running program, a
selected row, an open log — so blocks naming the same scene run in the same
session. That is what naming a scene means: the screen a beat leaves behind
is the screen the next one starts from, and a walkthrough with six narrated
steps is six blocks sharing one terminal, not six fresh shells.

It follows that a clip is not named by its own tape. `Type "j"` appears
twice in most walkthroughs — once to move down a list of pipelines, once to
move down a list of tasks — and those are one tape and two pictures. So
each beat publishes a `capture_key` alongside its `span_hash`: the hash of
its own source and every source before it in the session, which is OCI's
chain ID, arrived at for the reason OCI needed one.

Invalidation is then arithmetic rather than a rule. Editing a beat changes
that beat's key and every key after it in its session; nothing before it,
and nothing in another scene. Moving a paragraph changes every later beat's
start time and no beat's key, because start time is not in one — a
container rebuilds by position, and this does not.

`session="…"` on a block names a different run of the scene, for a script
that quits a program and starts it again:

````markdown
```teleprompt scene=terminal session=retry
Type "flowrs"
Enter
```
````

The name is not in the key: a run that opens with the same tape opens on the
same screen, so it is the same picture and the same clip.

## The manual narrates itself

`manual/` is a teleprompt project whose script is this tool's command-line
manual. Its prose is the narration; its action blocks are tapes running the
commands the prose is describing.

```bash
cargo run -- check manual/scripts/cli.md
cargo run -- plan  manual/scripts/cli.md
cargo run -- diff  manual/scripts/cli.md
```

Its `[scene.terminal]` block puts `target/release` on the capture shell's
`PATH`, so the tapes run the binary the checkout just built — the manual
demonstrates the tool by using it, which is only true if the terminal in
the video is running it.

`manual/timelines/cli.en.json` is committed, and
`crates/teleprompt-cli/tests/manual.rs` recompiles the manual on every CI run
and fails when it no longer matches — so a command that grows a flag cannot
leave the sentence about it, or the tape demonstrating it, quietly behind.

That timeline is compiled from a **cold cache** on purpose: every narration
duration in it is a word-count estimate, which any machine reproduces with
nothing installed. Dubbing the manual locally is what makes it audible:

```bash
cargo run -- dub manual/scripts/cli.md --out manual/build/narration
```

With `voice.backend = "null"` that writes silence of the estimated length —
enough to check the pacing. Point `teleprompt.toml` at a Kokoro server to hear
the words.

Dubbing warms `manual/.teleprompt/cache`, so `diff` will afterwards report
every segment as `now measured` against the committed cold-cache timeline.
That is the documented drift, not a regression, and it is why the test
compiles the manual in a scratch directory instead of in place.

## Voice backends

`voice.backend` in `teleprompt.toml` (or a script's front matter) picks which
backend renders narration. It defaults to `null`, which produces silence of
the estimated duration and needs nothing installed — that is what keeps a
fresh `cargo run -- new demo` working with no setup. The other backend this
build ships is `kokoro`, which speaks HTTP to a
[Kokoro-FastAPI](https://github.com/remsky/Kokoro-FastAPI) server you run
yourself, in Docker or via pip. teleprompt owns no Python and bundles no
model.

```toml
[voice]
backend = "kokoro"
voice = "af_heart"
speed = 1.0

[backends.kokoro]
base_url = "http://localhost:8880"
timeout_ms = 30000
concurrency = 4
model = "kokoro"
```

`base_url` is the server's address; teleprompt does not start or manage it.
`timeout_ms` (default 30000) bounds each synthesis request. `concurrency`
(default 4) bounds how many segments `dub` sends at once — a local model
server is the bottleneck, so unbounded fan-out only makes it slower.
`model` (default `"kokoro"`) is sent as the request's `model` field, for
servers hosting more than one checkpoint.

**Kokoro only has to be running for `dub` to succeed.** `check`, `plan`, and
`diff` read durations out of the cache — a real measurement once a segment
has been synthesized, a word-count estimate otherwise — and never make a
network call, never start an async runtime, and never open a socket. That is
why the whole inner loop (edit prose, `plan`, `diff`, repeat) works on a
laptop with no Kokoro, no Docker, and nothing else installed. `dub` (and
`dub --check`) is the one command that fails without a reachable server.
`doctor` also starts a runtime and opens a socket — it probes whichever
backend the project has configured — but a down server there is a warning,
not a failure; see below.

**`base_url` and `model` are part of the cache key.** Pointing `dub` at a
different server or a different model changes what produced the audio, so
teleprompt re-synthesizes every segment once and caches the result under the
new key — the old entries are still on disk, just no longer addressed by
anything. This falls out of treating the key as "everything that changes the
audio," but it has a sharp edge worth knowing in advance: `localhost` and
`127.0.0.1` are two different strings, so retargeting `base_url` from one to
the other — even though they may be the exact same server — is a full
re-dub, not a no-op. Use whichever spelling you intend to keep using.

**A broken server fails the run.** An unreachable, slow, or non-200 Kokoro
fails `dub` with exit 1 naming the URL and the segment, rather than
substituting silence. The voice fallback ladder moves between tiers an
author explicitly asked for (`recorded` → `cloned` → `synthetic`); a
synthesizer that is simply down is not a tier, and silently swapping in
silence for a voice would be the worst possible failure mode for a tool
whose whole point is narration.

`doctor` probes whichever backend the project has configured and reports a
down server as a warning, not an error — `check` and `plan` do not need it
reachable, so a project's steady state is `dub`-only downtime, not a broken
build.

### Running the real-server test

Everything above is covered by tests that run in CI against an in-process
HTTP stub — no real Kokoro, no network, no model download. One test is
different: `crates/teleprompt-voice-kokoro/tests/real.rs` hits an actual
server and is `#[ignore]`d so CI never touches it. To run it yourself, start
a Kokoro-FastAPI server on `localhost:8880` and then:

```bash
cargo test -p teleprompt-voice-kokoro --test real -- --ignored
```

It lists the server's voices, synthesizes one sentence, and asserts the
audio's sample rate, channel count, and duration are plausible.

## Building and testing

No network, no browser and no Node are required — the whole workspace
builds and tests offline. ffmpeg is needed by `build` and by the render
tests, which skip themselves where there is none; CI installs one and sets
`TELEPROMPT_REQUIRE_FFMPEG=1`, which turns that skip into a failure:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

`tests/fixtures/tour.md` is a realistic multi-chapter script exercised by
`crates/teleprompt-cli/tests/end_to_end.rs`, the acceptance suite that
answers M0's defining question: does editing prose produce a legible, useful
diff of the video's pacing?
