<picture>
  <source media="(prefers-color-scheme: dark)" srcset="apps/icons/teleprompt-lockup-dark.svg">
  <img alt="teleprompt" src="apps/icons/teleprompt-lockup-light.svg" width="360">
</picture>

# teleprompt

Compile narrated videos from version-controlled Markdown.

A script's prose is its narration, and its fenced `teleprompt` blocks are
what the screen shows. The length of the speech sets the pacing, so editing
a paragraph changes the rhythm of the video, and `teleprompt plan --check`
shows exactly how before anything renders.

````markdown
# Quick start

Every video here is built from a script you can read. {#welcome}

```teleprompt scene=vhs
Type "teleprompt plan scripts/tour.md"
Enter
Sleep 2s
```
````

teleprompt schedules the script, synthesizes the narration, records each
scene with the tool that already owns it, and renders the result with
ffmpeg:

| scene | recorded with |
|---|---|
| terminals | [VHS](https://github.com/charmbracelet/vhs) tapes, or [asciinema](https://asciinema.org) recordings |
| browsers | [Playwright](https://playwright.dev) scripts |
| motion graphics | an existing [Remotion](https://www.remotion.dev) project |
| slides | an existing [Slidev](https://sli.dev) deck |
| images, clips, title cards | ffmpeg alone |

Narration comes from a local [Kokoro](https://github.com/remsky/Kokoro-FastAPI)
server, a [Voicebox](https://voicebox.sh) voice made from your own takes,
Google's [Gemini TTS](docs/guide/voices.md#gemini-tts) with your API key,
or the built-in `null` voice, which is silent but correctly timed and
needs nothing installed.

## Try it

```bash
cargo run -- new demo
cargo run -- plan demo/scripts/demo.md
cargo run -- plan --check demo/scripts/demo.md
```

`new` scaffolds a project with a demo script. `plan` compiles it into a
timeline and prints one row per item, with its narration length and
policy. `plan --check` compares that timeline with the committed one,
and exits 3 when they differ. On a fresh
project nothing is committed yet, so every item shows as `added`.

Commit the timeline, edit a paragraph, and run `plan --check` again to watch the
transitions move:

```bash
cargo run -- plan demo/scripts/demo.md --format json > demo/timelines/demo.en.json
# edit demo/scripts/demo.md
cargo run -- plan --check demo/scripts/demo.md
```

`plan` and `plan --check` are instant and offline, whatever voice you configure.
When you want to hear the result, `serve` gives you a live preview that
opens on the item you just changed, and `build` renders the video.

## Commands

| command | does |
|---|---|
| `new <path>` | scaffold a new project |
| `from <doc>` | draft a script from a Markdown document or Slidev deck you already have |
| `record <script>` | record yourself working (asciinema, VHS or Playwright) while you talk, and get a script spoken in your voice (opt-in build) |
| `translate <script> --to <locale>` | translate the narration, for a video in another language |
| `import <recording>` | the same, from a cast or tape and a recording of your voice you already have |
| `check <script>` | parse and validate; no side effects, no cost |
| `plan <script>` | compile the timeline and print it; `--check` compares it with the committed one and exits 3 on drift |
| `serve <script>` | serve a live preview that opens on the item that changed |
| `prompt <script>` | a prompter that follows your voice, plays its shots as you reach them and records your takes (opt-in build) |
| `dub <script> --out <dir>` | synthesize narration and write audio plus a manifest |
| `capture <script>` | record the scenes a build will show |
| `build <script>` | render the video |
| `doctor` | report the environment teleprompt can see |
| `voice clone <name>` | make a voice from your takes on a [Voicebox](docs/guide/voices.md#your-own-voice-voicebox) server, so unrecorded lines sound like you |
| `setup [adapter or tool…]` | say which tools and models are installed, their licenses, and how to install the rest; `--run` installs them |
| `cache` | report what the project's caches hold, or shrink them |

Every command accepts `--format json`. Exit codes are `0` for success, `1`
for a runtime failure, `2` for an invalid script and `3` for drift.

## Documentation

- [Writing scripts](docs/guide/scripts.md): lines and blocks, policies,
  cues, transitions, configuration, and drafting from a document
- [Scenes](docs/guide/scenes.md): capture, sessions, and each adapter
- [Voices](docs/guide/voices.md): backends, the voice cache, and Kokoro
- [Reading from a prompter](docs/guide/prompter.md): `prompt`, which
  follows your voice
- [Translating a video](docs/guide/translating.md): `translate` and
  `--locale`, for the same video in another language
- [Recording a session](docs/guide/recording.md): `record` and `import`,
  which draft a script from a terminal session you narrated
- [Rendering](docs/guide/rendering.md): `build`, `dub` and the manifest,
  and the live preview
- [Rendering with Remotion](docs/integrations/remotion.md): consuming the
  manifest from a Remotion project
- [Design](docs/design.md): how it works and why, for contributors
- [Contributing](CONTRIBUTING.md): building, testing, and the rules for
  changes

## The manual narrates itself

`manual/` is a teleprompt project whose script is this tool's command-line
manual. Its prose is the narration, and its action blocks are tapes running
the commands the prose describes, using the binary this checkout built.

```bash
cargo run -- plan manual/scripts/cli.md
cargo run -- dub  manual/scripts/cli.md --out manual/build/narration
```

`manual/timelines/cli.en.json` is committed, and CI recompiles the manual
and fails when it no longer matches, so a command that changes can't leave
its page behind. The committed timeline is compiled from a cold cache,
with estimated durations any machine reproduces. Dubbing the manual warms
the cache, so `plan --check` then reports every line as `now measured`. That drift
is expected.

`examples/` holds a complete project for each of Remotion, Slidev,
asciinema and media scenes, and `demos/flowrs` is a two-scene walkthrough
of real software.

## License

MIT. See [LICENSE](LICENSE).
