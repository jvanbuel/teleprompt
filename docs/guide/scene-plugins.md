# Writing a scene plugin

A scene plugin brings a kind of scene a script can show: a terminal, a
browser, slides, a desktop app. It wraps a tool that already exists: it
has the tool's own language for a block's body, compiles it into shots
offline, and records them with the tool. (A voice is not a plugin: it is a
[speech server](voices.md#writing-a-voice).)

A project configures a scene plugin into scenes: `[scene.demo] plugin =
"vhs"` with a theme and a size, and `[scene.wide]` with another. A block
names the scene, and the scene names its plugin.

The ones in this repository are examples as much as features: every one is
written against the same crate an outside plugin would use,
`teleprompt-scene`, and nothing else of teleprompt's.
`tools/check_deps.py` holds them to that.

## The contract

A scene plugin is a scene compiler, a capture backend and optionally a
recorder, handed over as one `teleprompt_scene::ScenePlugin`, named by the
compiler's `kind()`. That name is what a scene's `plugin` gives.

```rust
pub fn plugin() -> teleprompt_scene::ScenePlugin {
    teleprompt_scene::ScenePlugin::new(MyScene, MyCapture::default())
        // .recorded_with(MyRecorder), if your tool records sessions
}
```

**The scene compiler runs offline.** `check`, `plan` and the prompter call
it, and none of them may start your tool. It validates a block's body,
reporting every bad line where it is (`validate`), and splits it into shots
at `mark` lines (`shots`), each with how long it takes: `Exact` when the
source states it, `Estimated` for a bound, `Unknown` when the shot should
last as long as its line (`Shot::lasting`). The defaults cover the rest:
`retime`, `continues`, `inputs`, `select`.

**The capture backend runs your tool.** It is given a whole session, the
shots that share a screen, because a walkthrough's shots continue one
another. It writes a clip for each wanted shot and returns them. What it
runs is its `needs`; by default it cannot run here when a program among
them is not on PATH, and `unavailable` can say more. `capture::Job` does
the rest of what every backend does: a scratch directory
(`work_dir`), where each clip goes and saying it is done (`clip_path`,
`keep`), a failure that names its shot (`failed`), and, for a tool that
records the whole session as one video, cutting it into clips
(`cut_reel`).

**The `media` module of `crates/teleprompt-scenes` is the smallest example**:
images and videos, captured with ffmpeg. `vhs` and `asciinema` record
sessions too. `examples/plugins/teleprompt-scene-card` is a complete
scene plugin in Python, as a program, and `examples/plugin-authors` is a
short video made with it.

## Shipping one

A plugin ships in one of two ways:

- **A program of its own**, in any language: `teleprompt-scene-<name>`,
  on PATH or in the plugins directory. Teleprompt finds it, starts it
  when a command needs it, and talks to it in JSON
  ([the protocol](#the-protocol)). Installing one needs no rebuild.
- **A Rust crate compiled into teleprompt**, as the ones in this
  repository are. The same crate can also be built as a program of its
  own ([a Rust plugin as a program](#a-rust-plugin-as-a-program)).

`teleprompt setup` lists every scene plugin in a table, built in or
installed as a program, with what it needs and what of that is missing
here; one that cannot be used says why. A second table lists the voices.
`teleprompt setup <name>` shows one plugin, with the tools it needs.

`teleprompt-scene` has a module per part of the contract:

| module | what you implement |
|---|---|
| `contract` | `SceneCompiler`: a block in your tool's own language, validated and split into shots, offline |
| `capture` | `CaptureBackend`: a session of shots run, and a clip kept for each |
| `record` | `Recorder`, optional: an author's working session recorded, for `teleprompt record` |
| `core::tool` | `Tool`, for what your plugin needs that teleprompt does not ship, and helpers to run it |
| `protocol` | a plugin as a program: teleprompt's side, and serving a Rust plugin |

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

`needs()` on the capture backend or the recorder lists them.
`tool::FFMPEG` and `tool::NODE` are shared; use those rather than
declaring your own. A server the author runs is `Found::Unknowable` with a
`guide` saying how to set it up.

## Registering it

**As a program:** put it on PATH, or in the plugins directory
(`$TELEPROMPT_PLUGINS`, or `teleprompt/plugins` in your data directory,
such as `~/.local/share/teleprompt/plugins`). Install it however suits its
language: `pipx`, `npm install -g`, `cargo install`, or a copied file. A
built-in plugin's name wins over an installed one's, which `teleprompt
plugins` says. Publish it with the GitHub topic `teleprompt-plugin`.

**Compiled in:** add a module to `crates/teleprompt-scenes`, written
against `teleprompt-scene` alone, and one line to `built_in()` in
`crates/teleprompt-registry/src/scenes.rs`, with its name in `SHIPPED`
there.

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

A plugin answers **`describe`** first:

```json
{"protocol": 1, "name": "card", "needs": [], "continues": false}
```

Its `name` matches its program's. A tool in `needs` is `{"name", "what",
"license", "home", "program" or "package", "install":{"brew": "...", "apt":
"...", ...}}`, which `teleprompt setup` lists and installs; a `program`
not on PATH is why the plugin cannot capture here. `continues` says
whether a shot opens on the screen the previous one left (true unless
said). Then it answers:

| method | params | result |
|---|---|---|
| `validate` | `scene`, `body` | `errors`: `[{line (from 0), message, help}]`, none for a good block |
| `shots` | `scene`, `body` | `shots`: `[{source, ms, exact}]`, split at `mark`; `ms` with `exact` when the source states its length, `ms` alone for an estimate, neither to last as long as its line |
| `retime` | `source`, `target_ms` | `source` rewritten to last exactly that long, or null; optional, and a plugin without it answers an error |
| `capture` | `session` (`scene`, `name`, `settings`, `shots`: `[{id, key, source, duration_ms, wanted}]`), `frame` (`width`, `height`, `fps`), `out_dir` | `clips`: `[{key, path}]`, one per wanted shot, with a progress event after each |

`validate` and `shots` must not start your tool: `check` and
`plan` ask them, offline. Teleprompt numbers the shots, hashes their
sources and places the errors in the script.

Not in version 1: recording a session, files outside a block that a scene
reads, and `include=…#fragment`. They are compiled-in only for now.

## A Rust plugin as a program

`teleprompt_scene::protocol::serve` serves a `ScenePlugin` over stdin and
stdout, so a crate written against the contract can be built as a program
too:

```rust
// src/bin/teleprompt-scene-mine.rs
fn main() -> std::io::Result<()> {
    teleprompt_scene::protocol::serve::scene(my_plugin::plugin())
}
```

## Testing it

The `mock` scene plugin (`teleprompt_scene::MockScene` and
`capture::mock::MockCapture`) shows the contract with no tool behind it.
A scene compiler is tested without its tool. A capture backend is tested
against the real tool where it is installed, and skips, saying so, where
it is not.
