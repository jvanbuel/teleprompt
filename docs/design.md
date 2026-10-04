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
paragraph moves every transition after it, and `plan` and `plan --check` show that
before anything renders. The one exception is a picture whose length is
fixed, such as a recording or an animation: an item can be led by its
picture instead ([`fit-line`](#led-by-the-picture)), and its length is the
picture's own. A length is written only for a shot that cannot state one.

**Narration is never cut.** Every policy but one takes the speech's length
as fixed and adjusts only the action. `voice.speed` is a synthesis
parameter the author sets, sent to the backend so the voice is generated
at that rate. The one policy that changes speech is `fit-line`, which the
author asks for on an item led by its picture: it speeds the line up or
slows it down, keeping its pitch, within bounds, and says how many words to
cut past them. Nothing cuts speech. When pacing is wrong, the fix is to
edit the prose.

**A video is a function of a committed script and a cache.** Every derived
artifact is content-addressed. An edit costs exactly the recomputation it
invalidates, and deleting any cache entry costs time, never correctness.

**The inner loop never awaits.** `check` and `plan` never
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

A paragraph that opens with a bold label, `**Guest:**` or `**Guest**:`,
names its speaker when the cast (`voices`) has someone of that name, by
slug (`Ada Lovelace` is `ada-lovelace`), and the label is not said. A
line whose speaker is not the last line's waits `timing.turn_gap_ms`
more before it starts. Otherwise the label is text and
is said, with a warning when there is a cast. Speakers are written the
way transcripts are, so a script renders as one anywhere Markdown does,
and it is resolve, which knows the cast, that decides, not the parser.

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

Lines take attributes in a trailing `{#id key=value}`, headings in the
same form (Pandoc's), and action blocks in the fence info string. The
grammar is shared, and an unknown key is an error. A heading's `#id` is
its chapter's slug, and its other keys are any setting, dotted. The keys a line takes are listed in
`teleprompt-core/src/attrs.rs` as `SEGMENT_KEYS`, and the keys a block
takes as `BLOCK_KEYS`.

### Line identity

Line ids anchor the voice cache, the manifest's audio files and the
diff. An explicit `{#id}` wins. Otherwise the id is
`<chapter-slug>-<n>`, where `n` is the line's position in its chapter, so it
shifts when a line is inserted before it. `teleprompt from` writes explicit
ids from the start for that reason. An action block's is its `id=`, or its
line's id and `-a` (`-a2` for a second block after the same line; `-b1`
for one before any line), counting every block so pinning one renames no
other. The parser gives every line and block its id, so none is ever
missing, and `resolve` checks them all. An id becomes a file name
(`audio/<id>.wav`) that a manifest consumer resolves, so ids are checked
against path traversal before anything is built from them.

### Configuration

Configuration layers merge in this order, with the last one winning:

1. built-in defaults
2. `teleprompt.toml` at the project root
3. the script's front matter
4. a chapter's heading settings, then its YAML block, immediately after
   the heading
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
| `teleprompt-voice-voicebox` | HTTP against a Voicebox server: a voice cloned from the author's takes, or designed from a description, and delivery instructions |
| `teleprompt-voice-gemini` | HTTP against Google's Gemini TTS (the Interactions API): prebuilt or custom voices, delivery instructions sent beside the words |
| `teleprompt-cache` | the content-addressed voice cache |
| `teleprompt-scene` | the `SceneCompiler` contract and the `mock` adapter |
| `teleprompt-capture` | the `CaptureBackend` contract and the mock backend, the `Recorder` contract for recording a session, and `Adapter`, the three under one name; re-exports `scene`, so an adapter depends on this crate alone |
| `teleprompt-compile` | where the others meet: walks a `Program`, drives voice and scene, emits items, and builds the manifest; and what `dub` publishes from each line's audio, which the prompter plays too |
| `teleprompt-manifest` | the manifest's types and its diff: what a renderer reads, without depending on how it was compiled |
| `teleprompt-listen` | following a reader through a script: aligns what a speech recognizer hears against the script's words, and fires cues as the reader reaches them; no dependencies |
| `teleprompt-listen-sherpa` | the recognizer, a streaming sherpa-onnx model; empty without its opt-in `sherpa` feature, so the default build stays offline |
| `teleprompt-derive` | drafting a script: from a recorded session (when each step began and timed words in, lines and the blocks between them out), a Markdown document, a Slidev deck's notes, or a conversation's transcript. Pure: no IO |
| `teleprompt-translate` | translation providers for `translate`, chosen by name: a local model through Ollama (the default), any OpenAI-compatible server, Claude, and a command of the author's; the request each is sent and the prompt the model-backed ones share |
| `teleprompt-lsp` | the language server: the protocol, positions and completion; what a script compiles to comes from an `Analyzer` the CLI implements, so it depends on core alone |
| `teleprompt-prompter` | the prompter as a library: a `Session` that follows a reader, says which shots to play, and records takes; knows nothing of HTTP |
| `teleprompt-render` | the ffmpeg renderer and its chunk cache; reads the manifest, not the compiler |
| `teleprompt-vhs`, `-asciinema`, `-playwright`, `-remotion`, `-slidev`, `-media` | one crate per adapter, holding its scene compiler, capture backend and, where its tool records (asciinema, VHS, Playwright), its recorder, handed over as one `adapter()` |
| `teleprompt-desktop` | what the desktop adapters share: their action language and scene compiler, and the runner that plays a session's shots against a window and cuts them where they began |
| `teleprompt-x11`, `-macos` | one crate per platform: the app on a virtual X display, driven by `xdotool`; or on the Mac's own screen, driven through JavaScript for Automation. Each records with ffmpeg |
| `teleprompt-cli` | the `teleprompt` binary, and the registries every adapter and backend is composed into |

`core`, `schedule`, `voice` and `scene` do not depend on one another.
Anything that needs two of them belongs in `compile`. `render` reads the
manifest and never the compiler. `scene` and `capture` stay two crates:
`compile`, and so `plan`, `check` and the prompter, depends on `scene`
alone, so it reads a scene's shots and cannot run its tool. An adapter
sees one crate all the same: `capture` re-exports `scene`, and each
adapter crate's `adapter()` hands over its compiler, capture backend and
recorder as one `Adapter`, named by its compiler's kind, so its halves
cannot be registered under two names. `tools/check_deps.py` holds the
allowed edges between workspace crates, and CI fails on any other.
Adapters and backends are registered only in the CLI, so adding one means
one crate and one registry line, and no other crate learns its name.

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

`check` and `plan` read durations from the voice cache's metadata
and never touch audio bytes. A hit is `measured`. A miss is the estimator's
prediction, published as `estimated`. `dub` and `build` synthesize every
miss, so everything they publish is `measured`.

The first `dub` after writing a line therefore moves the timeline. `plan --check`
reports this as `now measured`, not `text edited`, because the two call
for different responses. `plan` warns when it emits estimates, since a
timeline committed from a cold cache drifts on the next dub.

### Async boundary

`compile()` takes a `VoiceContext`: a cache, an estimator and resolved
settings. None of these can reach a backend, so no expression on the
`check` or `plan` path can call `synthesize`. The boundary is
structural. `main` is synchronous, and only `dub`, `build`, `capture`,
`prompt` (for its voice) and `doctor` build a runtime.

### Voice cache

`.teleprompt/cache/voice/<key>.wav`, with a JSON sidecar that holds the
duration, format and word timings. The key covers everything that changes
the audio and nothing else: the backend id, the backend version, locale,
voice, speed, the hash of the text as synthesized, with pronunciations
applied, and the delivery instructions (`voice.instruct`) when there are
any. A field that is absent adds nothing to the key, so a new one never
changes the keys of a cache built before it.

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
`audio_hash`, so recording a line again shows in `plan --check`. The timeline marks
the line `recorded`. Editing the line leaves its take behind, and the line
is synthesized again until it is re-recorded. Once a project has any
takes, every command that compiles names the lines it still synthesizes.

A take kept by the prompter also holds what the recognizer heard in it,
`heard`: the take is transcribed again whole when it stops, so no word is
heard cut at a line's edge, and its words are split among the lines it
read. Where they are other words than the line's, the prompter offers the
line as it was said (`teleprompt_core::said`): the script's own words
with their capitals and punctuation, the words heard where they differ,
and a dropped sentence end moved to the word before. A small streaming
model mishears, so a word a letter off in three, or heard split in two
("a round"), counts as the script's word and offers nothing. `teleprompt edit <script>
said <line>` writes it into the script and gives the take the new words,
so it stays current: keeping what you said instead of reading it again.

`prompt` records takes. The prompter is `teleprompt-prompter`, a
`Session` with typed calls (`start`, `listen`, `stop`, `script`, `clip`);
`teleprompt prompt` serves it as an API and the page that drives it.
**The page is the only prompter.** The apps (`apps/linux`, `apps/macos`)
show it in WebKit and add what a page cannot have: a window per screen,
the microphone's permission, a welcome page, settings, setup, and on
Linux a terminal for session mode. Each prompter feature is built once,
in the page; the apps never learn of one. The page streams the microphone at
its own rate; the session keeps that as the take and feeds the recognizer a
16 kHz copy.
When the take is kept, it is cut at the silence between lines, found
between where the follower heard one line end and the next begin, and each
line read in full is saved.

`dub` publishes a take converted to the rate and channels of the
synthesized lines, because the manifest has one audio format. The
conversion keeps the length to the millisecond, since the manifest
publishes it. A take whose bytes no longer match their sidecar fails the
command. The prompter plays a recorded line from its take.

#### Prompter API, version 1

Everything is under `/api/v1`, on loopback. The script and clips are plain
HTTP; the session is a WebSocket, since the microphone is a stream.
Loopback keeps other machines out but not other web pages, which a browser
lets reach it, WebSockets included. So this server
answers 403 to a `Host` other than `127.0.0.1` or `localhost` (a rebound DNS
name) and to an `Origin` other than its own page; a client that is not
a page sends no `Origin`. With
`--format json` and `--port 0`, the command picks a free port and prints
`{"event":"listening","url","api"}` on stdout once it listens, so an app
that launched it knows where to load the page from. `docs/api/v1/examples`
has one of each message, and the server is tested against them. The page
takes settings in its address: `?countdown=0` starts a take without a
count of three, `?shell=1` leaves the script's name to an app's title bar,
and `?view=monitor` is the screen alone, which the page opens in a window
of its own for a second display and keeps in step with it.

| route | does |
|---|---|
| `GET /api/v1/script` | `{"lines":[{"id","text","recorded","stale","said","said_diff"}],"shots":[{"shot","at":{"line","word"},"clip"}]}`; `clip` is a URL, or null if the shot was never captured. `stale` marks a line reworded since its take, due to be recorded again. `said` is the line as its take was heard to say it, where that is other words, or null: `keep_said` on the socket, or `teleprompt edit <script> said <line>`, keeps it. `said_diff` is the line against `said` word by word, runs of `{"kind":"same"|"gone"|"new","words"}`, empty without it: what keeping it changes, for a prompter to show. If the script's file changed since last asked, it is reloaded first, between takes: a shot moved, a line reworded. For a project's script it also has `"voice":{"name","listens"}`, who reads it and whether the prompter follows a reader by ear (false under `--voice`); `"length_ms"`, the video's length as the timeline has it now; and on each line `"speaker"`, who says it from the script's cast, or null for the narrator; `"instruct"`, how its voice is told to say it, or null; and `"audio":{"source":"take"|"voice","url","ready","duration_ms","words"}`: where the line's audio comes from, whether it is made yet, and once it is, its length and when each word starts, in milliseconds (the voice's own timings where it gives one per word, spread over the line's characters otherwise). And `"timeline":{"duration_ms","lines":[{"id","start_ms","end_ms"}],"shots":[{"shot","block","scene","line","start_ms","end_ms","timed"}]}`, the scheduler's plan as the page draws it on the glass in Edit mode: each line from its first sound to its last, and each shot with the line it runs with or after, and whether it states its own length, so can be stretched. And `"error"`: the compile errors of the script as last saved, while it does not compile and the rest is the last version that did, or null |
| `GET /api/v1/clips/<key>.mp4` | a cued shot's clip; nothing else in the cache |
| `GET /api/v1/voice/<line id>.wav` | the line's audio: its current take, or its voice's, synthesized now into the voice cache if it is not there yet. `?fresh=1` synthesizes it again, for a voice that says a line differently each time. `?fit=1` is the line as the manifest publishes it, the file `dub` writes: in the video's format, and a `fit-line` line at its tempo |
| `GET /api/v1/manifest` | the manifest `dub` would publish (`narration.json`, version 3), with every line synthesized first; the page plays the video from it. While the script does not compile it is the last one that did; if it never has, 422 with `{"ok":false,"errors"}` |
| `GET /api/v1/session` | the session socket; one at a time, a second gets 409 |
| `POST /api/v1/make?job=capture\|build` | runs `teleprompt capture` or `build` on the script, as the command would be run, and answers with what it reports as it goes: each of its [progress events](#cli), one JSON object a line, then `{"event":"made","job","video"}` (`video` null for a capture) or `{"event":"failed","job","errors"}`. One at a time, a second gets 409 |

On the socket, the client sends:

- `{"type":"start","from":N,"rate":HZ}`: a new take at line N, with audio
  at HZ; a take not stopped is dropped.
- binary messages: little-endian f32 mono samples at the take's rate.
- `{"type":"stop"}`: keep the lines read in full. A take a line already
  had is put aside, not lost.
- `{"type":"discard"}`: end the take and keep none of it.
- `{"type":"undo"}`: put back what the last take kept replaced, each
  line's take before it or none; once, until another take is kept.
- `{"type":"keep_said","line":id}`: reword the line to what its take was
  heard to say, as `teleprompt edit <script> said <line>` does. Between
  takes; the script's next fetch reads the new words.
- `{"type":"reword","line":id,"text"}` and
  `{"type":"instruct","line":id,"text"}`: say the line in other words, or
  tell its voice how to say it (a null or empty `text` removes the
  instruction), as `teleprompt edit` does: written only if the script
  still compiles.
- `{"type":"cue","block":id,"word":N}`, `{"type":"hold","block":id}`,
  `{"type":"move","block":id,"after":line id,"word":N|null}` and
  `{"type":"stretch","block":id,"by":X}`: a shot dragged on the glass, as
  `teleprompt edit` makes each, and as carefully.
- `{"type":"undo_edit"}`: put the script back as it was before the last
  edit. The server keeps it as it was before each, and refuses once the
  file has been changed since by anything else: an edit made in the
  author's editor is never undone over.

The server sends:

- `{"type":"reached","line","word","play":[shot]}` after a start, and
  whenever audio moves the reader or reaches a shot. `line` and `word` are
  the next word to be said, words counted by splitting the line's text at
  whitespace.
- `{"type":"stopped","saved":[line id]}`.
- `{"type":"discarded"}`.
- `{"type":"undone","lines":[line id]}`: the lines put back.
- `{"type":"kept_said","line":id}` once the line is reworded.
- `{"type":"edited","line":id}` once a `reword` or `instruct` is written,
  and `{"type":"edited","block":id}` once a shot's edit is.
- `{"type":"edit_undone"}`.
- `{"type":"error","message"}` for a message it did not understand or a
  take it could not save or line it could not reword; the session goes on.
  Under `--voice`, where the script's voice reads it, a `start` is one.

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

### Recording a session

Recording is an adapter's, like compiling and capturing: `record --with
<adapter>` has the adapter's own tool record the author (`asciinema rec`
with its keystrokes, `vhs record`, `playwright codegen`), while ffmpeg
records the microphone; `import` takes a recording and a voice from
elsewhere. A `Recorder` starts the tool, stops it as its own stop would
(the recorded shell hung up, `vhs` sent SIGTERM), and reads what it wrote
back as steps: when each began, its part of the file, and the mark that
cuts the file before it in the adapter's scene. A cast's step is a command,
from its first key to the next command's; a tape's, a command through its
`Enter`, timed as `vhs` would play it; a codegen script's, a statement,
timed by when it appeared in the file, which the recorder watches. The
voice is placed on the recording's clock by when its first sample was
taken. `teleprompt-derive` then drafts the script, purely, from the steps'
start times and the words the recognizer heard with their times:

- Speech is cut into lines where a silence reaches 700 ms. The prose is
  verbatim, since the result is a first draft. With a punctuation model,
  the whole transcript is punctuated once and each word takes its
  punctuated form back, matched by its letters, so no word's time moves
  and a word the punctuator rewrote is kept as heard. Without one, a
  recognizer that hears in capitals is lowered. Either way each line
  starts with a capital and ends a sentence.
- Each step is placed by when it began. Started during a line (or up to
  150 ms before it), it runs `concurrent` with the line, and if it started
  more than 600 ms in, it is cued to the phrase being said then, grown
  until it occurs once in the line. Started in a pause, it holds after the
  line before. Steps placed alike, one after another, are one block.
- The recording is saved beside the script, in `recordings/`, with a mark
  before each block's first step, and each block is an
  `include=recordings/<script>.<ext>#<part>` of it: the author's own
  session in the tool's own format, not a translation of it. The command
  that closed the shell is left out.

`import` compiles what it wrote to learn its line ids, and saves each
line's stretch of the recording as its [take](#takes): from 150 ms before
its first word to 250 ms after its last, never past halfway to the next
line.

### Translation

A script is written in one locale, `locales.source`. Compiling it for
another (`--locale nl`) reads the translation beside it,
`tour.nl.yaml` beside `tour.md`, and puts it in place of the source's
text before anything is compiled: each line, each chapter title, and
each cue. Scenes, tapes and every other setting are the source video's,
over which `[locale.nl]` in configuration applies, a Dutch voice for
instance.

Each entry records the first twelve hex digits of the hash of the text
it was translated from. So the file is its own staleness ledger: an
entry whose source has changed is spoken, and reported as out of date,
until it is translated again. A line with no entry at all is an error,
since there is nothing to say in its place. The translated text hashes
as itself, so its audio is keyed apart from the source line's, and a
take recorded for the source line is never spoken in its place.

A cue names a phrase of its line, which the translation mostly does not
contain. It is taken from the file's `cues:`; failing that it is kept
when the translation kept its words, as it keeps a command; and failing
that the shot starts on the words as far into the translated line as the
phrase was into the source, with a warning.

`translate` fills the file in, with the provider `[translate]` names. The
default runs a model locally through Ollama, so, like speech recognition
and synthesis, translation needs no service. A provider is a name, its own
settings under `backends.<name>` as a voice backend's are, and a
`translate` method; `openai` and `command` reach anything else without a
new build. It asks only for what is missing or
stale, and for the cues of lines it retranslates, and sends what is
already translated along so terminology holds. Answers are merged in
script order, and entries for lines the script no longer has are
dropped. A translated cue that is not words of its line is left out for
compile to place.

## Scheduling

The scheduler is a pure function from items, with their durations, to a
`Timeline`. It does no IO and reads no clock.

### Policies

| policy | effect |
|---|---|
| `hold` (default) | The line plays over the picture left by the item before it, and the action runs after the line ends. |
| `concurrent` | The action and the line overlap. `align=start` (the default), `end` or `center` places the shorter within the longer. A [cue](#cues) moves the action to a phrase. |
| `fit-action` | The action is re-timed, slower or faster, to last exactly as long as the line, within `min_stretch` and `max_stretch`. The bound is applied with a warning. |
| `trim-action` | An action longer than its line is cut to the line's length, with a warning when it is more than `trim_warn_above` times the line's length. A shorter action is left alone. |
| `fit-line` | The line is sped up or slowed down to fit the action, which keeps its length: see [led by the picture](#led-by-the-picture). |

A policy only changes a picture if the adapter can re-time its source (see
`retime` under [the scene contract](#scene-contract)). Where the adapter
cannot, the renderer holds the last frame for the rest of the slot.

`align` on any policy but `concurrent` is an error naming the combination,
since it would change nothing. Renamed spellings are errors that name
their replacement, never aliases, so scripts converge on one name:
`stretch` and `stretch-action` (it compresses too) became `fit-action`,
`trim` became `trim-action`, and `max_speedup` (it only ever set when a
trim is reported) became `trim_warn_above`, as a block attribute and as a
`timing` key.

### Led by the picture

Each item is led by its line or by its picture, and its policy says which;
the line leads by default. Items follow one another, so a script mixes
both: a slide held for as long as its line is spoken, then a recorded
terminal session the next line must fit.

`fit-line` makes the picture lead. The action's length is the item's, and
the line's clip, between its lead-in and tail, is played at the tempo that
fits it there, between `min_line_speed` and `max_line_speed` (0.9 and
1.15), or `min_take_speed` and `max_take_speed` (0.95 and 1.08) for a
recorded take, since stretching a real voice is more audible. Past a bound
the tempo stops at it, and a warning says by how much the line is over or
under, and, when over, about how many of its words to cut. A line shorter
than its picture at the slowest tempo ends early, and the picture plays on.

The action's length must be known: an `Exact` or `Estimated` duration, or
`budget=` on the block for a shot whose length is `Unknown`, such as a
Playwright script. `fit-line` without either is an error, and so is
`budget=` on a shot that states its own length, since the two could
disagree. `cue=` and `align=` are errors with `fit-line`: they place an
action against its line, and here the line is placed against the action.

The timeline and the manifest record the tempo as `tempo_permille` on the
narration, absent when it is 1000, and the narration's duration is its
length at that tempo. `dub` writes each line's audio at its tempo,
stretched in time by `teleprompt_voice::stretch`, which keeps pitch; the
cache keeps the voice as it was synthesized or recorded, so changing a
tempo costs no synthesis.

`timing.length_ms`, in front matter or `teleprompt.toml`, is the whole
video's: `check` warns when the timeline runs over it, saying how much of
the time is fixed by pictures and how much is narration, and about how
many words to cut.

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
timestamps and no floats, so the file is byte-stable. In code, a start is
a `TimeMs` and a duration a `SpanMs`, so one is never added to or passed
for the other, and a `fit-line` tempo is a `Tempo`, which is never the
1000‰ that means no change.

### Diff

`plan --check` compares the computed timeline with the committed one and names a
reason for each change: `text edited`, `now measured`, `audio changed`, and
so on. It also reports what shifted as a consequence. `--exit-code` exits
3 on drift, which is what lets CI fail a pull request that changed prose
without re-planning.

## Scenes

A scene is a running instance of an adapter: one session, with its
settings. A fence names it (`scene=vhs`). An adapter's name is a scene
without being declared; declaring one (`[scene.server] adapter = "vhs"`)
gives it settings, or a second instance of an adapter under another name.
The block body is adapter-native, so a scene's name does not abstract
over tools: moving a block to another adapter means rewriting it. A name
that is neither declared nor an adapter is an error, not a placeholder.
`terminal` and `browser` are kept as older names for `vhs` and
`playwright`.

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
  where the source does not state its timing. This is how `fit-action`
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
- **`x11`** and **`macos`**: a desktop app, run from the scene's `command`
  and driven by a block of actions (keys, typing, the pointer, `Wait` on the
  window's title), in VHS's vocabulary. The language, its timing and
  `retime` are `teleprompt-desktop`'s, so the two platforms compile a block
  alike and differ only in how they run it. Timing is `Exact` without a
  `Wait`, which makes it `Estimated`. An app's own time is not predictable,
  so these cut shots differently from `vhs`: the recording is stamped with
  the wall clock (`-use_wallclock_as_timestamps`), the runner notes the
  clock as each shot begins and holds each to its slot, and a shot is cut
  where it began. A slow launch or a slow redraw moves a shot's start, never
  its picture.
- **`mock`**: scripted durations, for testing the compiler's side of the
  contract.

## Manifest

`teleprompt dub --out <dir>` writes `<dir>/<locale>/narration.json` and
`audio/<line>.wav` beside it. The manifest is the contract for anything
that renders the video: teleprompt's own `build` and prompter, and outside
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

A speaker's name, drawn over their first line, is drawn into the chunks it
falls over and is part of their keys: its text and where it starts and
ends, counted from the chunk's first frame, which is also what places its
fade. A chunk no one is named over keys as it did before names existed.
The name comes from the manifest, each line's `name`, so another renderer
can draw names its own way.

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
`--format json`, and errors are reported in the format requested. With it,
a long command's progress is one JSON event per line on stderr, for an app
to show as it goes: `{"event": "progress", "stage": "voice" | "capture" |
"render", …}` with `done` and `of` (lines, shots) or `done_ms` and `of_ms`
(a render). The report is on stdout at the end.

### Exit codes

| code | meaning |
|---|---|
| 0 | success |
| 1 | runtime failure (IO, network, a subprocess) |
| 2 | validation error: the script, its config or its arguments |
| 3 | drift, from `plan --check` or `dub --check` |

A validation error names the file, line and column, quotes the line, and
says what was expected.

With `--format json`, every report is an object with `ok`, true exactly
when the exit code is 0, and a failure is `{"ok": false, "errors": [...]}`.
Documents are printed as they are written and carry no `ok`: the timeline
`plan` prints, the manifest `dub` writes, and `record --tools`' list.

## Testing

- The scheduler, parser, ids, config, cache keys and diff are pure, and
  their tests need nothing installed.
- The `null` voice and the `mock` scene and capture backends are real
  implementations of their contracts. With them, `check`, `plan`
  and `dub` run end to end offline.
- `crates/teleprompt-cli/tests/golden.rs` pins `plan` for every example
  project, and every command's failure output and exit code.
- `crates/teleprompt-cli/tests/page` drives the prompter page in Chromium
  against a real `teleprompt prompt`, with a recorded reading as the
  microphone: a take followed and kept, shots dragged and a drag undone, a
  build, a reworded line re-recorded. `apps/linux/tests/ui.sh` drives the
  Linux app around it under a virtual display.
- `manual/timelines/cli.en.json` is compared against a fresh compile of
  the manual, so a command that changes cannot leave its manual page
  behind.
- Kokoro is tested against an in-process HTTP stub. One `#[ignore]`d test
  uses a real server.
- Render and capture tests need ffmpeg or the adapter's tool. They skip
  where it is missing, and CI sets `TELEPROMPT_REQUIRE_*` so a skip there
  is a failure.

## What teleprompt ships

Teleprompt is MIT, and everything in its binaries must let it stay that
way, so that anyone can install, bundle or ship it without a lawyer. What
a video needs comes in three tiers.

1. **Built in.** Every adapter, voice backend and recognizer above is
   teleprompt's own code, in every build; `listen` is a feature only
   because its native library is large. The crates they link are checked in
   CI (`deny.toml`, `cargo deny check licenses`) to be permissively
   licensed: a crate under the GPL, or any license not on the list, fails
   the build.
2. **Tools, detected and never shipped.** What an adapter runs, `vhs`,
   Playwright, `asciinema`, `ffmpeg`, `Xvfb`, `xdotool`, Remotion and Slidev, and what a
   backend talks to, a Kokoro server or a speech model, are programs and
   files of their own, under their own licenses: `asciinema` and some
   `ffmpeg` builds are GPL, and Remotion needs a company license past a
   team's size. Teleprompt runs the copy the author installed, as a
   separate program, and links none of them. A hosted service, Gemini TTS
   or a translator's API, is the author's own account, used only when
   chosen. `doctor` says which are
   missing, and `teleprompt setup <adapter or tool>` says how to install
   each with the author's own package manager, and its license, and runs
   that command when asked (`--run`). Models go in
   `$TELEPROMPT_MODELS`, by default `teleprompt/models` in the user's data
   directory, where `prompt`, `record` and `import` look when no
   `--model` is given. What the author's own project holds, a Remotion
   project or a Kokoro server, `setup` explains rather than installs.
   `setup` is organised by use (render, terminal, browser, prompt,
   drafts, conversations…), since that is what an author knows they
   want; `setup --uses` reports them for the apps, which install through
   the same command and read its progress events. The apps add nothing:
   what a use needs and how it installs stay the CLI's. **No release artifact, whether binary, app bundle, installer
   or container image, includes a tool.** One that did would take on the
   tool's license: a deliberate choice, made in this document first.
3. **Community plugins, not yet built.** Adapters and voices of other
   people's, fetched by the author from their own repositories, as Herdr
   and Obsidian do: an executable and a manifest, speaking JSON on stdin
   and stdout, installed with a command and listed by a GitHub topic.
   Teleprompt would host nothing and pass no license on.

## Not built

Named here so the rest of this document is not read as covering them:

- **Cloning with Gemini.** `voice clone` makes a Voicebox voice from the
  author's takes. Google's Voices API replicates a voice too, but only with
  a recording of its owner reading a consent statement, which teleprompt
  does not collect; a voice made there is named by its id.
- **Runtime-loaded plugins.** Backends and adapters are compiled in; see
  [What teleprompt ships](#what-teleprompt-ships) for the plan.
