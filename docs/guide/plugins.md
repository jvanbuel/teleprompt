# Writing a plugin

Teleprompt's scenes and voices are plugins. Each tool a script can show (a
terminal, a browser, slides, a desktop app) is an **adapter**, and each way
a line can be spoken is a **voice**. The ones in this repository are
examples as much as features: every one is written against the same crate
an outside plugin would use, `teleprompt-plugin`, and nothing else of
teleprompt's but `teleprompt-core`. `tools/check_deps.py` holds them to
that.

Today a plugin is a Rust crate compiled into `teleprompt`. Plugins as
separate programs, which an author installs without rebuilding, are next
(see [What comes next](#what-comes-next)).

## The crate

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

Add your crate to the workspace, and one line to `teleprompt-cli`:
`adapters()` in `src/scene.rs` for an adapter, `plugins()` in
`src/voice.rs` for a voice. Then add your crate to `tools/check_deps.py`,
with `PLUGIN` as what it may depend on.

## Testing it

The `mock` adapter (`teleprompt_plugin::scene::MockScene` and
`capture::mock::MockCapture`) shows the contract with no tool behind it.
A scene compiler is tested without its tool. A capture backend is tested
against the real tool where it is installed, and skips, saying so, where
it is not.

## What comes next

Plugins as programs of their own, fetched by the author: an executable and
a manifest, speaking JSON on stdin and stdout, installed with a command and
listed by a GitHub topic. Then a plugin can be written in any language, and
installing one needs no rebuild. The built-in plugins will be its first
examples (`docs/design.md#what-teleprompt-ships`).
