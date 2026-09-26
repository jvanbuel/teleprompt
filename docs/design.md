# teleprompt design

How teleprompt works and why it is shaped this way. The README says how to
use it; this document is for changing it. It describes the code as it is.
Plans live in the issue tracker, and history lives in commit messages.

Code cites this document by anchor, as `docs/design.md#quiet-window`, so a
heading here is an interface. Rename one only together with the comments
that link to it.

## Principles

**Timing is derived, not authored.** A line's spoken length is its
duration, and the visuals are paced to it. Other programmatic video tools
have the author write durations. Here the author writes prose. Editing a
paragraph moves every transition after it, and `plan` and `diff` show that
before anything renders.

**Narration is never altered.** Every policy takes the speech's length as
fixed and adjusts only the action. Nothing resamples, compresses or cuts
speech. `voice.speed` is a synthesis parameter the author sets, sent to the
backend so the voice is generated at that rate. The scheduler never writes
it. When pacing is wrong, the fix is to edit the prose.

**A video is a function of a committed script and a cache.** Every derived
artifact is content-addressed. An edit costs exactly the recomputation it
invalidates, and deleting any cache entry costs time, never correctness.

**The inner loop never awaits.** `check`, `plan` and `diff` never
synthesize, read audio or open a socket. They stay instant and offline
whatever the configured voice or scene needs (see
[the async boundary](#async-boundary)).

**Adopt the tool that owns the domain.** A terminal scene is a VHS tape, a
browser scene is a Playwright script, and a slide is a Slidev deck.
teleprompt defines a contract and wraps these tools. It does not define a
step language of its own.

**Fail loudly, degrade visibly.** Input that is unknown or has no effect is
an error, not ignored. A voice server that is down fails the command rather
than turning into silence. Where degrading is the right call, as with a
scene this machine cannot record, the result says so: the shot renders as a
slate and `build` reports how many slates it rendered.

## Vocabulary

| term | meaning |
|---|---|
| **script** | A Markdown source file. The unit of compilation. |
| **chapter** | A heading and everything under it. |
| **line** | One narration paragraph, with a stable id. The unit of voice. |
| **action block** | A fenced `teleprompt` block of adapter-native source. |
| **scene** | A domain a video can show (`terminal`, `browser`, `media`). What a fence names. |
| **adapter** | The implementation serving a scene (`vhs`, `playwright`). Configuration, not part of the script. |
| **mark** | A point inside an action block where narration may interleave. |
| **shot** | The part of an action block between marks. The unit of capture. |
| **item** | A line, the shot that follows it, or both. The unit of scheduling. |
| **session** | One continuous run of a scene. Shots in a session continue one another. |
| **timeline** | The compiled schedule, committed as `timelines/<script>.<locale>.json`. |
| **manifest** | `narration.json`, the published contract a renderer reads. |

## Script format

Front matter configures, headings are chapters, and paragraphs are lines.
Fenced blocks with the `teleprompt` info string are action blocks. The
document is parsed as CommonMark, and any other node (lists, tables, other
code fences) is ignored by compilation, so a script can also be ordinary
documentation.

A paragraph that is only an HTML comment is a directive, not narration.
`<!-- teleprompt: pause 800ms -->` inserts a silent item. Inline markup is
normalised for speech: emphasis is stripped, and code spans and links read
as their text.

**An action block's body belongs to its adapter.** teleprompt reads the
fence attributes to learn the scene and hands the body to that scene's
adapter to validate. It never interprets the body itself. Adding a scene
therefore never changes the Markdown grammar. `include=<path>` takes the
body from a file, so a tape or a Playwright spec stays a real file that its
own tooling can run. `include=<path>#<fragment>` selects part of that file.
What a fragment means is the adapter's to decide (see
[`select`](#scene-contract)).

### Attributes

Lines take attributes in a trailing `{#id key=value}`, and action blocks
take them in the fence info string. The grammar is shared, and an unknown
key is an error. The keys a line takes are listed in
`teleprompt-core/src/attrs.rs` as `SEGMENT_KEYS`, and the keys a block
takes as `BLOCK_KEYS`.

### Line identity

Line ids anchor the voice cache, the manifest's audio files and the
diff. An explicit `{#id}` wins. Otherwise the id is
`<chapter-slug>-<n>`, where `n` is the line's position in its chapter, so it
shifts when a line is inserted before it. `teleprompt from` writes explicit
ids from the start for that reason. An id becomes a file name
(`audio/<id>.wav`) that a manifest consumer resolves, so ids are checked
against path traversal before anything is built from them.

### Configuration

Configuration layers merge in this order, with the last one winning:

1. built-in defaults
2. `teleprompt.toml` at the project root
3. the script's front matter
4. a chapter's YAML block, immediately after its heading
5. line or block attributes
6. command-line flags

Backend settings live in a generic map, `backends.<id>`, that core never
interprets. Each backend deserializes its own entry, so a new backend needs
no field in `teleprompt-core`. That map is read only from
`teleprompt.toml`, and a script that repeats it in front matter gets a
warning.

## Pipeline

```
parse ─► resolve ─► voice ─► schedule ─► capture ─► compose
└──────────── plan ──────────────┘   └────── render ─────┘
```

1. **Parse**: Markdown to AST. Assign ids, validate attributes, and hand
   each block to its adapter to validate and split into shots. `check`
   stops here.
2. **Resolve**: merge configuration and resolve each line's voice. The
   result is a `Program`.
3. **Voice**: get each line's duration, from the cache or the estimator in
   the inner loop, or by synthesis in `dub` and `build`.
4. **Schedule**: apply policies and produce the `Timeline`. `plan` stops
   here.
5. **Capture**: run each scene as one session and keep a clip for each
   shot that does not have one.
6. **Compose**: place clips and narration, then encode.

`dub` runs stages 1 to 4 with real synthesis and publishes the
[manifest](#manifest). `build` runs `dub`, then capture, then compose,
reading the manifest it just published.

### Crates

| crate | holds |
|---|---|
| `teleprompt-core` | AST, parser, ids, config, `Hash`, diagnostics, shared vocabulary such as `VoiceSource` |
| `teleprompt-schedule` | policies, the scheduler, `Timeline`, diff. Pure. |
| `teleprompt-voice` | the `VoiceBackend` and `DurationEstimator` contracts, WAV encoding, the registry |
| `teleprompt-voice-null` | silence at the estimated length, and `WpmEstimator` |
| `teleprompt-voice-kokoro` | HTTP against a Kokoro-FastAPI server |
| `teleprompt-cache` | the content-addressed voice cache |
| `teleprompt-scene` | the `SceneCompiler` contract and the `mock` adapter |
| `teleprompt-capture` | the `CaptureBackend` contract and the mock backend |
| `teleprompt-compile` | where the others meet: walks a `Program`, drives voice and scene, emits items, and builds the manifest |
| `teleprompt-manifest` | the manifest's types and its diff: what a renderer reads, without depending on how it was compiled |
| `teleprompt-listen` | following a reader through a script: aligns what a speech recognizer hears against the script's words, and fires cues as the reader reaches them; no dependencies |
| `teleprompt-listen-sherpa` | the recognizer, a streaming sherpa-onnx model; empty without its opt-in `sherpa` feature, so the default build stays offline |
| `teleprompt-prompter` | the prompter as a library: a `Session` that follows a reader, says which shots to play, and records takes; knows nothing of HTTP |
| `teleprompt-render` | the ffmpeg renderer and its chunk cache; reads the manifest, not the compiler |
| `teleprompt-vhs`, `-asciinema`, `-playwright`, `-remotion`, `-slidev`, `-media` | one crate per adapter, holding its scene compiler and capture backend |
| `teleprompt-cli` | the `teleprompt` binary, and the registries every adapter and backend is composed into |

`core`, `schedule`, `voice` and `scene` do not depend on one another.
Anything that needs two of them belongs in `compile`. `render` reads the
manifest and never the compiler. `tools/check_deps.py` holds the allowed
edges between workspace crates, and CI fails on any other. Adapters and backends
are registered only in the CLI, so adding one means one crate and one
registry line, and no other crate learns its name.

## Voice

### Voice contract

A backend does one thing: it turns text into audio.

```rust
#[async_trait]
pub trait VoiceBackend: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> VoiceCapabilities;
    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError>;
}
```

The trait does not return a duration separately. The audio's length is the
duration, so a backend cannot report a length its samples do not have.
`Pcm` carries its own sample rate and channel count, and nothing
downstream assumes one. `capabilities().version` is the backend's own
version, such as the model it runs, and it is part of the cache key.

Predicting a duration without synthesizing is a separate trait,
`DurationEstimator`, which is synchronous and deterministic.
`WpmEstimator` (words per minute, punctuation-aware) is the only
implementation. Backend authors never implement it.

### Estimated and measured

`check`, `plan` and `diff` read durations from the voice cache's metadata
and never touch audio bytes. A hit is `measured`. A miss is the estimator's
prediction, published as `estimated`. `dub` and `build` synthesize every
miss, so everything they publish is `measured`.

The first `dub` after writing a line therefore moves the timeline. `diff`
reports this as `now measured`, not `text edited`, because the two call
for different responses. `plan` warns when it emits estimates, since a
timeline committed from a cold cache drifts on the next dub.

### Async boundary

`compile()` takes a `VoiceContext`: a cache, an estimator and resolved
settings. None of these can reach a backend, so no expression on the
`check`, `plan` or `diff` path can call `synthesize`. The boundary is
structural. `main` is synchronous, and only `dub`, `build`, `capture`,
`serve` and `doctor` build a runtime.

### Voice cache

`.teleprompt/cache/voice/<key>.wav`, with a JSON sidecar that holds the
duration, format and word timings. The key covers everything that changes
the audio and nothing else: the backend id, the backend version, locale,
voice, speed, and the hash of the text as synthesized, with pronunciations
applied.

The backend version is the *model*, not the server address, so a cache
built on one machine is valid on another. The trade-off is that two
servers serving different weights under one model name collide. `doctor`
shows the model beside the address so a mismatch is visible. teleprompt's
own version is never part of the key, because a release that changed every
key would report "audio changed" on every line of every consumer's next
drift check.

A corrupt or truncated entry reads as a miss with a reason, and it is
overwritten on the next synthesis. Nothing ever needs a manual clean.

Each file is written to a temporary path and renamed into place, audio
before sidecar, and lookups key off the sidecar, so an interrupted store
reads as a miss. Two `dub` processes storing one key could still
interleave into a valid sidecar beside audio it doesn't describe, so
publishing the audio is also an exclusive claim. One writer wins and the
other adopts its entry.

### Takes

A line can be spoken from a recording instead of synthesized. Recordings
live in `takes/` at the project root: `<line>.wav`, and a sidecar
`<line>.json` holding the text the line had when it was read, the
recording's length and the hash of its bytes. They are source, not cache,
since nothing can make them again, so they are committed.

A take is *current* while its line still reads exactly as it did when it
was recorded. A current take is the line's voice: its length is measured
from the sidecar, so `plan` reads no audio, and its hash is the line's
`audio_hash`, so recording a line again shows in `diff`. The timeline marks
the line `recorded`. Editing the line leaves its take behind, and the line
is synthesized again until it is re-recorded. Once a project has any
takes, every command that compiles names the lines it still synthesizes.

`prompt` records takes. The prompter is `teleprompt-prompter`, a
`Session` with typed calls (`start`, `listen`, `stop`, `script`, `clip`);
`teleprompt prompt` serves it as an API and the page that drives it, so
another front end, such as a native app, can drive the same session through
the API or call the library directly. The page streams the microphone at
its own rate; the session keeps that as the take and feeds the recognizer a
16 kHz copy.
When the take is kept, it is cut at the silence between lines, found
between where the follower heard one line end and the next begin, and each
line read in full is saved.

`dub` publishes a take converted to the rate and channels of the
synthesized lines, because the manifest has one audio format. The
conversion keeps the length to the millisecond, since the manifest
publishes it. A take whose bytes no longer match their sidecar fails the
command. `serve` plays a recorded line from its take.

#### Prompter API, version 1

Everything is under `/api/v1`, on loopback. The script and clips are plain
HTTP; the session is a WebSocket, since the microphone is a stream. With
`--format json` and `--port 0`, the command picks a free port and prints
`{"event":"listening","url","api"}` on stdout once it listens, so an app
that launched it knows where to connect. `docs/api/v1/examples` has one of
each message; the server and the macOS app (`apps/macos`) are both tested
against them.

| route | does |
|---|---|
| `GET /api/v1/script` | `{"lines":[{"id","text","recorded"}],"shots":[{"shot","at":{"line","word"},"clip"}]}`; `clip` is a URL, or null if the shot was never captured |
| `GET /api/v1/clips/<key>.mp4` | a cued shot's clip; nothing else in the cache |
| `GET /api/v1/session` | the session socket; one at a time, a second gets 409 |

On the socket, the client sends:

- `{"type":"start","from":N,"rate":HZ}`: a new take at line N, with audio
  at HZ; a take not stopped is dropped.
- binary messages: little-endian f32 mono samples at the take's rate.
- `{"type":"stop"}`: keep the lines read in full.

The server sends:

- `{"type":"reached","line","word","play":[shot]}` after a start, and
  whenever audio moves the reader or reaches a shot. `line` and `word` are
  the next word to be said, words counted by splitting the line's text at
  whitespace.
- `{"type":"stopped","saved":[line id]}`.
- `{"type":"error","message"}` for a message it did not understand or a
  take it could not save; the session goes on.

Within a version, the API only grows: new fields, messages and routes.
Clients ignore what they do not know. A change that would break a client
is `/api/v2`.

### Backend failure

A backend that is unreachable, slow or returns an error fails the command
with exit 1, naming the URL and the line. It never falls back to `null`.
The ladder moves between tiers an author asked for, and a broken server is
not a tier. `doctor` reports a down server as a warning, because only `dub`
needs it.

### Word timings

A backend that advertises `word_timings` returns when each word is said.
The manifest then publishes `words`, and [cues](#cues) land on the word
instead of being interpolated. Kokoro serves timings only from a `/dev/`
endpoint, so they are opt-in (`backends.kokoro.word_timings`), and turning
them on changes the backend version (`<model>+words`) so no cached entry
without timings answers for one with them.

`voice.pronounce` maps written words to how they should be said. It
applies to synthesis only. Captions, the manifest and the script keep the
spelling, and because the mapped text is in the cache key, correcting a
pronunciation re-renders exactly the lines that use that word.

## Scheduling

The scheduler is a pure function from items, with their durations, to a
`Timeline`. It does no IO and reads no clock.

### Policies

| policy | effect |
|---|---|
| `hold` (default) | The line plays over the picture left by the item before it, and the action runs after the line ends. |
| `concurrent` | The action and the line overlap. `align=start` (the default), `end` or `center` places the shorter within the longer. A [cue](#cues) moves the action to a phrase. |
| `stretch-action` | The action is re-timed to last exactly as long as the line, within `min_stretch` and `max_stretch`. The bound is applied with a warning. |
| `trim-action` | An action longer than its line is cut to the line's length, with a warning above `max_speedup`. A shorter action is left alone. |

A policy only changes a picture if the adapter can re-time its source (see
`retime` under [the scene contract](#scene-contract)). Where the adapter
cannot, the renderer holds the last frame for the rest of the slot.

The names `stretch-action`, `trim-action` and `max_speedup` do not describe
what they do. `stretch-action` also compresses, and `max_speedup` is a
warning threshold, not a speed. Renaming them is decided but not done:
issue #15. The old spellings `stretch` and `trim` are errors that name
their replacement.

### Cues

`cue="phrase"` on a `concurrent` block starts the action when the line
reaches that phrase. When the line was synthesized with word timings, the
action starts on the phrase's first word, matched against the text as the
voice was given it. Otherwise the start is interpolated from where the
phrase sits in the text, by characters rather than bytes. A cue that names
something the line does not say is an error, and so is a cue on any policy
other than `concurrent`.

### Padding

`lead_in` and `tail` (150 ms each by default) pad every line, so speech
never butts against a transition. A cue's offset is measured from the first
word, after the lead-in.

### Quiet window

With `transition.duration: auto`, the transition into the next item is
`clamp(slack × 0.5, min_ms, max_ms)`, where slack is the line's duration
minus the action's. It is then capped at the *quiet window*: this item's
trailing silence plus the next item's lead-in. That is the time around the
boundary when nobody is speaking.

Without the cap, a script of plain paragraphs has no action to subtract,
so every transition runs to `max_ms` and crossfades over real speech. The
cap overrides `min_ms`, because a shorter transition, or none, is better
than one that talks over the narration. A fixed duration is honoured as
written, with a warning when it exceeds the quiet window.

### Timeline

`timelines/<script>.<locale>.json` is committed, and it is the review
surface. A reviewer reads it, not the video, to see what a prose change did
to the pacing. For each item it records start and duration, the policy,
the line's hashes, the shot's scene, adapter, `shot_hash`,
`capture_key` and duration source, and the transition. There are no
timestamps and no floats, so the file is byte-stable.

### Diff

`diff` compares the computed timeline with the committed one and names a
reason for each change: `text edited`, `now measured`, `audio changed`, and
so on. It also reports what shifted as a consequence. `--exit-code` exits
3 on drift, which is what lets CI fail a pull request that changed prose
without re-planning.

## Scenes

A scene is a domain and an adapter is a tool. A fence names the scene
(`scene=terminal`), and configuration chooses the adapter
(`[scene.terminal] adapter = "vhs"`). The block body is adapter-native
either way. The indirection keeps the choice of tool out of every script,
so swapping it is a configuration change, not a find-and-replace.

### Scene contract

`SceneCompiler` is the compile-time half. It is synchronous and does no
IO, which is why `check` and `plan` stay offline however heavy the tool is.

- `validate` checks a block and reports every bad line, with positions in
  the script or in the included file.
- `shots` splits a validated block at its marks.
- `estimate` returns `Exact` for a source that states its own timing,
  `Estimated` for a bound, and `Unknown` for a source that cannot say. A
  shot of unknown length takes its line's length.
- `retime` rewrites a shot so it lasts a target length, or returns `None`
  where the source does not state its timing. This is how `stretch-action`
  reaches inside a tape.
- `continues` says whether a shot opens on the screen the previous shot
  left behind. It is true by default, and false for adapters whose shots
  depend only on their own source.
- `inputs` and `shot_inputs` name files outside the block that the picture
  is drawn from, such as a project or an image. The compiler reads and
  hashes them into the [capture key](#capture-key), so the contract itself
  stays free of IO.
- `select` returns the part of an included body that a `#fragment` names.
  The default refuses the fragment and names the adapter.

`CaptureBackend` is the runtime half. It runs one session, writes a clip
for each shot it is asked for, and reports through `unavailable()` when
this machine cannot run it.

### Marks

A mark is spelled as a comment in the adapter's own language (`# mark` in
a tape, `// mark` in a Playwright script), so the body stays a file its
tool can run unchanged. Without marks, a block is one shot and narration
cannot interleave with it.

### Capture key

The shots of a walkthrough continue one another, so the screen at shot *n*
is the accumulation of the shots before it in the same session. A clip is
therefore not named by its own source, because the same keystroke can
produce two different pictures. The key is a chain, in the same way as
OCI's chain ID:

```
name(n)  = H(recipe, adapter, scene config, inputs, shot source)
chain(0) = H(name(0))
chain(n) = H(chain(n-1) ‖ name(n))
```

Invalidation follows from the arithmetic. Editing a shot changes that
shot's key and every later key in its session, and nothing before it or in
another scene. Moving a paragraph changes start times but no key, because
position is not in the key. Slot length enters through `retime`: a re-timed
shot's source is the source that will be captured. An adapter whose
`continues` is false names each shot by `name(n)` alone.

The *recipe* (`CAPTURE_RECIPE` in `teleprompt-compile`) versions how
teleprompt draws a picture: the recorder's version, the tape or script it
writes around a shot, a changed default. Bump it when the picture for the
same script changes, since neither the adapter name nor the author's
settings would move. `inputs` are the scene's `inputs` and the shot's
`shot_inputs`, hashed by content.

A block joins its scene's default session. `session="…"` starts another
session, for a script that quits a program and starts it again. The
session's name is not in the key.

### Capture

Capture runs each session once and files a clip under each shot's capture
key in `.teleprompt/cache/video/`. A shot whose clip is cached still runs,
because the next shot opens on the screen it leaves. A session with nothing
left to capture is not opened, and a session stops after the last shot it
needs. A scene this machine cannot record produces a warning and a slate,
not a failure.

### Adapters

- **`vhs`**: a VHS tape, run by `vhs`. `Sleep` and `TypingSpeed` make a
  shot's timing exact, and a shot with `Wait` is `Estimated` at its
  timeout. `retime` rewrites the tape's pauses. Commands that would
  conflict with teleprompt's capture are refused, each with a reason:
  `Output`, `Set Shell`, `Source`, `Env`, `Screenshot` and
  `Set PlaybackSpeed`. So is any setting it does not recognise, since a
  misspelled setting would otherwise change a video while `check` passes.
  One `vhs` run records a whole session, and shots are cut from it.
- **`asciinema`**: a recorded cast, for sessions that should not run again
  at capture time. Exact, split at the cast's own markers, and `select`
  takes `#2`, `#2-3` or `#label`.
- **`playwright`**: a Playwright script, run as written. Its duration is
  `Unknown` and it cannot be re-timed, so it takes its line's length and
  holds the last frame if it finishes early.
- **`remotion`**: a composition from an existing Remotion project, with its
  props. It is always exactly as long as asked, and `continues` is false.
  The project's files are its `inputs`.
- **`slidev`**: a slide of an existing Slidev deck at a click step
  (`3?clicks=2`), exported as a still. Its length is not in the key, so
  rewording a line reuses its slide.
- **`media`**: `image`, `clip` and `title` directives, rendered with
  ffmpeg. Stills take their line's length. A ranged clip is exact and never
  re-timed, and its sound is dropped. Each shot's file is in its own key
  through `shot_inputs`.
- **`mock`**: scripted durations, for testing the compiler's side of the
  contract.

## Manifest

`teleprompt dub --out <dir>` writes `<dir>/<locale>/narration.json` and
`audio/<line>.wav` beside it. The manifest is the contract for anything
that renders the video: teleprompt's own `build`, `serve`, and outside
tools such as Remotion. `docs/integrations/remotion.md` is the consumer's
guide.

It carries `manifest_version` (3), provenance, `duration_ms`, the audio
format, `chapters`, `lines` (id, text, chapter, start, duration, duration
source, audio path, `source_hash`, `audio_hash`,
and `words` when timed), and `shots` (the action schedule with scene,
adapter, policy, transition, `shot_hash`, `capture_key` and session). A
consumer must refuse a version it does not know.

`audio_hash` hashes the WAV bytes on disk, not the voice cache key, so it
changes only when the audio does.

### Authoritative durations

A line's length is its own `duration_ms`, never the next line's `start_ms`
minus its own. Items, pauses and concurrent actions open gaps between
lines, and a fixed transition wider than the quiet window makes lines
overlap.

### Frame rounding

A consumer converts absolute offsets to frames, `round(ms × fps / 1000)`,
and takes differences. A line spans from `frames(start_ms)` to
`frames(start_ms + duration_ms)`. Rounding durations on their own lets the
error accumulate over a long video.

### Drift

The manifest is byte-stable: fields are in a fixed order, and there are no
timestamps and no floats. `dub --check` recomputes it and exits 3 on any
difference, so a pull request that changes prose without re-dubbing fails
CI. Commit the manifest. The audio can be regenerated, at the cost of
needing a voice backend wherever you render.

## Rendering

`build` reads the manifest it just published, exactly as an outside
consumer does. Two timing paths would drift, and the one that drifts
silently is the one nobody renders from. Frame size and rate come from
configuration, because they are not timing facts.

ffmpeg runs as a subprocess with an explicit argument vector. Linking libav
would put its headers in every build, and a GPL-configured ffmpeg inside an
MIT binary.

A shot with no clip holds its slot as a slate. Skipping it would run every
later shot early against narration that is still correctly placed.

### Compose cache

The picture is cut into chunks that can be encoded independently: a
placement, or the blend between two placements. Each chunk is cached under
a key that covers everything that changes its frames: its content, its
length, the frame size and rate, and the encoder settings. Chunks are
joined with the `concat` demuxer and `-c copy`. A chunk's key holds its
duration, not its position, so a line that grows moves everything after it
without invalidating it. The cache lives in `.teleprompt/cache/compose/`,
capped at 1 GB by default and evicted least recently used first.
`--no-cache` switches reuse off without switching renderers.

Audio is mixed in one pass over the whole timeline. It is cheap, and a mix
cut into chunks would put a join in the middle of a word. Output audio is
48 kHz stereo AAC, whatever rate the voice produced, because that is what
players handle reliably.

## Caches

| directory | holds | key |
|---|---|---|
| `.teleprompt/cache/voice/` | synthesized lines | [voice cache](#voice-cache) |
| `.teleprompt/cache/video/` | captured clips | [capture key](#capture-key) |
| `.teleprompt/cache/compose/` | encoded chunks | [compose cache](#compose-cache) |

All three are derived and gitignored. Entries are written to a temporary
path and renamed into place, so a partial write is never read as a hit. A
capture backend writes into a staging directory, and its clips are renamed
into the cache only once its whole session has succeeded.

## CLI

The binary is a thin shell over the library crates. Every command takes
`--format json`, and errors are reported in the format requested.

### Exit codes

| code | meaning |
|---|---|
| 0 | success |
| 1 | runtime failure (IO, network, a subprocess) |
| 2 | validation error: the script, its config or its arguments |
| 3 | drift, from `diff --exit-code` or `dub --check` |

A validation error names the file, line and column, quotes the line, and
says what was expected.

## Testing

- The scheduler, parser, ids, config, cache keys and diff are pure, and
  their tests need nothing installed.
- The `null` voice and the `mock` scene and capture backends are real
  implementations of their contracts. With them, `check`, `plan`, `diff`
  and `dub` run end to end offline.
- `crates/teleprompt-cli/tests/golden.rs` pins `plan` for every example
  project, and every command's failure output and exit code.
- `manual/timelines/cli.en.json` is compared against a fresh compile of
  the manual, so a command that changes cannot leave its manual page
  behind.
- Kokoro is tested against an in-process HTTP stub. One `#[ignore]`d test
  uses a real server.
- Render and capture tests need ffmpeg or the adapter's tool. They skip
  where it is missing, and CI sets `TELEPROMPT_REQUIRE_*` so a skip there
  is a failure.

## Not built

Named here so the rest of this document is not read as covering them:

- **Cloned voices.** There is no voice enrollment: a line is recorded or
  synthesized.
- **Localization sidecars.** `--locale` selects configuration, but
  translated narration files and their staleness ledger do not exist.
- **Deriving a script from a live session.** `from` drafts from documents
  and Slidev decks, not from recorded screen sessions.
- **Runtime-loaded plugins.** Backends and adapters are compiled in.
- **Narration-constrained scheduling**, where a fixed picture sets a
  budget and over-long prose becomes a diagnostic.
