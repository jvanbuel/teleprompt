# teleprompt

Compile narrated videos from version-controlled Markdown.

A script's prose is its narration; fenced `teleprompt` blocks are its visuals.
Narration duration drives visual pacing, so editing a paragraph changes the
rhythm of the video — and `teleprompt diff` tells you exactly how before you
render anything.

**Status: M0.** The compiler and the feedback loop work end to end: parsing,
narration timing, and timeline scheduling are all real. Terminal scenes are
written as [VHS](https://github.com/charmbracelet/vhs) tapes and compile to
exact durations. There is no video output yet — no rendering, no terminal
capture, no ffmpeg: teleprompt reads a tape, it does not yet run one. See
`docs/superpowers/specs/2026-08-15-teleprompt-design.md` for the full design
and milestone plan.

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
| `check <script>` | parse and validate; no side effects, no cost |
| `plan <script>` | compile the timeline and print it |
| `diff <script>` | compare against the committed timeline |
| `dub <script> --out <dir>` | synthesize narration and write audio plus a manifest |
| `serve <script>` | live preview that opens on the beat you just changed |
| `doctor` | report the environment teleprompt can see |

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

teleprompt ships no Remotion code and takes no Remotion dependency. Remotion's
own licence — free for individuals and organisations up to three employees —
is between you and Remotion.

## Terminal scenes

An action block whose `scene=terminal` is a [VHS](https://github.com/charmbracelet/vhs)
tape — Charm's recording language, borrowed whole:

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

teleprompt reads the tape language rather than shelling out to `vhs`, because
VHS renders a whole tape to one finished file and cannot pause in the middle,
and the middle is exactly where narration goes. M0 ships the reading half:
`check` validates a tape, `plan` and `diff` schedule it. Running one — the PTY
and the terminal capture — is M6.

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

## The manual narrates itself

`manual/` is a teleprompt project whose script is this tool's command-line
manual. Its prose is the narration; its action blocks are tapes running the
commands the prose is describing.

```bash
cargo run -- check manual/scripts/cli.md
cargo run -- plan  manual/scripts/cli.md
cargo run -- diff  manual/scripts/cli.md
```

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

No network, no browser, no Node, and no ffmpeg are required — the whole
workspace builds and tests offline:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

`tests/fixtures/tour.md` is a realistic multi-chapter script exercised by
`crates/teleprompt-cli/tests/end_to_end.rs`, the acceptance suite that
answers M0's defining question: does editing prose produce a legible, useful
diff of the video's pacing?
