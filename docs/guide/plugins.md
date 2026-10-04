# Writing a plugin

Teleprompt's scenes and voices are plugins. Each tool a script can show (a
terminal, a browser, slides, a desktop app) is an **adapter**, and each way
a line can be spoken is a **voice**. The ones in this repository are
examples as much as features: every one is written against the same crate
an outside plugin would use, `teleprompt-plugin`, and nothing else of
teleprompt's but `teleprompt-core`. `tools/check_deps.py` holds them to
that.

A plugin ships in one of two ways:

- **A program of its own**, in any language: `teleprompt-adapter-<name>`
  or `teleprompt-voice-<name>`, on PATH or in the plugins directory.
  Teleprompt finds it, starts it when a command needs it, and talks to it
  in JSON ([the protocol](#the-protocol)). Installing one needs no rebuild.
  `examples/plugins` has two, in Python: a scene of colour cards, and a
  voice spoken by eSpeak NG.
- **A Rust crate compiled into teleprompt**, as the ones in this
  repository are. The same crate can also be built as a program of its
  own ([a Rust plugin as a program](#a-rust-plugin-as-a-program)).

`teleprompt plugins` lists the plugins installed as programs, asks each
what it is, and says why one cannot be used.

`examples/plugin-authors` is a short video about all of this, made with
the two example plugins: its cards are drawn by the card adapter, and one
of its lines is spoken by the eSpeak voice.

## The contracts

`teleprompt-plugin` has one module per contract:

| module | what you implement |
|---|---|
| `scene` | `SceneCompiler`: a block in your tool's own language, validated and split into shots, offline |
| `capture` | `CaptureBackend`: a session of shots run, and a clip kept for each |
| `record` | `Recorder`, optional: an author's working session recorded, for `teleprompt record` |
| `voice` | `VoiceBackend`: a line of text turned into audio |
| `tool` | `Tool`, for what your plugin needs that teleprompt does not ship, and helpers to run it |

## An adapter

An adapter is a scene compiler, a capture backend and optionally a
recorder, handed over as one `Adapter`, named by the compiler's `kind()`.
That name is what a block's `scene=` gives.

```rust
pub fn adapter() -> teleprompt_plugin::Adapter {
    teleprompt_plugin::Adapter::new(MyScene, MyCapture::default())
        // .recorded_with(MyRecorder), if your tool records sessions
}
```

**The scene compiler runs offline.** `check`, `plan` and the prompter call
it, and none of them may start your tool. It validates a block's body,
reporting every bad line where it is (`validate`), splits it into shots at
`mark` lines (`shots`), and says how long each takes: `Exact` when the
source states it, `Estimated` for a bound, `Unknown` when the shot should
last as long as its line (`estimate`). The defaults cover the rest:
`retime`, `continues`, `inputs`, `select`.

**The capture backend runs your tool.** It is given a whole session, the
shots that share a screen, because a walkthrough's shots continue one
another. It writes a clip for each wanted shot and returns them. It says
why it cannot run here (`unavailable`), usually with `tool::missing`.

**`crates/teleprompt-media` is the smallest example**: images and videos,
captured with ffmpeg. `teleprompt-vhs` and `teleprompt-asciinema` record
sessions too.

## A voice

A voice implements `VoiceBackend`: `id`, `capabilities`, and `synthesize`,
which returns PCM. The other methods have defaults. Override them for what
your server can do: `concurrency` (lines sent at once), `address` (where
the server is), `voices` (checked before a dub), `probe` (the line `setup`
prints) and `clone_voice`.

It is registered as a `VoicePlugin`: its id, which `voice.backend` names,
and how it is built from its own `[backends.<id>]` settings in
`teleprompt.toml`.

```rust
pub fn plugin() -> VoicePlugin {
    VoicePlugin {
        id: "mine",
        build: |settings| Ok(Arc::new(MyVoice::new(MyConfig::from(settings)?)?)),
        needs: &NEEDS,
    }
}
```

A build that fails is kept and reported only where that voice is chosen,
so a broken block does not stop a project that uses another voice.
`capabilities().version` is part of the voice cache's key: put in it
anything that changes the audio but is not in the request, such as the
model.

**`crates/teleprompt-voice-kokoro` is the example**: HTTP to a server the
author runs.

## What it needs

A plugin never ships the tools it runs. It declares them, and
`teleprompt setup` lists each with its license and installs it with the
author's own package manager:

```rust
pub static MYTOOL: Tool = Tool {
    name: "mytool",
    what: "records the screen for my scenes",
    license: "MIT",
    home: "https://example.org/mytool",
    guide: None,
    found: Found::Program("mytool"),
    install: &[
        (Manager::Brew, "brew install mytool"),
        (Manager::Apt, "sudo apt-get install -y mytool"),
    ],
    download_mb: None,
};
```

`needs()` on the capture backend, the recorder or the `VoicePlugin` lists
them. `tool::FFMPEG` and `tool::NODE` are shared; use those rather than
declaring your own.

## Registering it

**As a program:** put it on PATH, or in the plugins directory
(`$TELEPROMPT_PLUGINS`, or `teleprompt/plugins` in your data directory,
such as `~/.local/share/teleprompt/plugins`). Install it however suits its
language: `pipx`, `npm install -g`, `cargo install`, or a copied file. A
built-in plugin's name wins over an installed one's, which `teleprompt
plugins` says. Publish it with the GitHub topic `teleprompt-plugin`.

**Compiled in:** add your crate to the workspace, and one line to
`teleprompt-cli`: `built_in()` in `src/scene.rs` for an adapter,
`plugins()` in `src/voice.rs` for a voice. Then add your crate to
`tools/check_deps.py`, with `PLUGIN` as what it may depend on.

## Testing it

The `mock` adapter (`teleprompt_plugin::scene::MockScene` and
`capture::mock::MockCapture`) shows the contract with no tool behind it.
A scene compiler is tested without its tool. A capture backend is tested
against the real tool where it is installed, and skips, saying so, where
it is not.

## The protocol

A plugin program reads requests from stdin and writes answers to stdout,
one JSON object a line. Its stderr is shown to the author, so it is where
a plugin logs. Teleprompt starts it the first time a command needs it,
keeps it for the rest of that command, sends one request at a time, and
stops it at the end.

```text
→ {"id":3,"method":"shots","params":{"scene":"card","body":"color red\nhold 2s"}}
← {"id":3,"result":{"shots":[{"source":"color red\nhold 2s","ms":2000,"exact":true}]}}
← {"id":4,"error":"why it could not"}
← {"id":5,"progress":{"shot":"intro-a#0","done":1,"of":2}}   (capture, before its result)
```

Every plugin answers **`describe`** first: `{"protocol":1, "kind":
"adapter" or "voice", "name", "needs":[tools]}`, its name matching its
program's. A tool is `{"name", "what", "license", "home", "program" or
"package", "install":{"brew": "...", "apt": "...", ...}}`, which
`teleprompt setup` lists and installs.

An **adapter** describes `"adapter":{"continues", "retimes"}` and answers:

| method | params | result |
|---|---|---|
| `validate` | `scene`, `body` | `errors`: `[{line (from 0), message, help}]`, none for a good block |
| `shots` | `scene`, `body` | `shots`: `[{source, ms, exact}]`, split at `mark`; `ms` with `exact` when the source states its length, `ms` alone for an estimate, neither to last as long as its line |
| `estimate` | `source` | `{ms, exact}`, as for a shot |
| `retime` | `source`, `target_ms` | `source` rewritten to last exactly that long, or null |
| `unavailable` | | `reason` it cannot capture here, or null |
| `capture` | `session` (`scene`, `name`, `settings`, `shots`: `[{id, key, source, duration_ms, wanted}]`), `frame` (`width`, `height`, `fps`), `out_dir` | `clips`: `[{key, path}]`, one per wanted shot, with a progress event after each |

`validate`, `shots` and `estimate` must not start your tool: `check` and
`plan` ask them, offline. Teleprompt numbers the shots, hashes their
sources and places the errors in the script.

A **voice** describes `"voice":{"word_timings", "speed_control", "address",
"lists_voices", "probes"}` and answers:

| method | params | result |
|---|---|---|
| `configure` | `settings`, its `[backends.<name>]` table, or null | `version`: what the audio depends on beyond the request, such as a model. Part of the voice cache's key, so never a path or a host |
| `synthesize` | `text`, `locale`, `voice`, `speed`, `instruct`, `out` | the line written to `out` as a WAV file; `word_timings` if it has them |
| `voices` | | `voices`: the names its `voice.voice` may give |
| `probe` | | `line`: one line on its server, for `setup` |

Not in version 1: recording a session, files outside a block that a scene
reads, `include=…#fragment`, and cloning a voice. They are compiled-in
only for now.

## A Rust plugin as a program

`teleprompt_plugin::protocol::serve` serves an `Adapter` or `VoicePlugin`
over stdin and stdout, so a crate written against the contracts can be
built as a program too:

```rust
// src/bin/teleprompt-adapter-mine.rs
fn main() -> std::io::Result<()> {
    teleprompt_plugin::protocol::serve::adapter(my_adapter::adapter())
}
```
