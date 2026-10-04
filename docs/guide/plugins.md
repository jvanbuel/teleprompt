# Extending teleprompt

There are two kinds of plugin, scene plugins and voice plugins:

- **A scene plugin** brings a kind of scene: a terminal, a browser,
  slides, a desktop app. It has a language of its own for a block's body,
  compiles it into shots offline, and records them into clips. A project
  configures it into scenes (`[scene.demo] plugin = "vhs"`), and a block
  names the scene: `scene=demo`. [Writing a scene plugin](scene-plugins.md).
- **A voice plugin** speaks a line. Most speech servers now speak OpenAI's API,
  and one of those needs **no plugin at all**: name it in `teleprompt.toml`
  ([any OpenAI-compatible server](voices.md#any-openai-compatible-server)).
  A voice that is not a server, such as a program on the author's machine,
  is a plugin. [Writing a voice](voices.md#writing-a-voice).

What the two share is plumbing: how a plugin ships, how it says what it
needs, and how teleprompt talks to it. That is this page.

The ones in this repository are examples as much as features: every one is
written against the same crate an outside plugin would use,
`teleprompt-plugin`, and nothing else of teleprompt's but `teleprompt-core`.
`tools/check_deps.py` holds them to that. `examples/plugins` has two
plugins written without it, in Python, and `examples/plugin-authors` is a
short video made with them.

## Shipping one

A plugin ships in one of two ways:

- **A program of its own**, in any language: `teleprompt-scene-<name>`
  or `teleprompt-voice-<name>`, on PATH or in the plugins directory.
  Teleprompt finds it, starts it when a command needs it, and talks to it
  in JSON ([the protocol](#the-protocol)). Installing one needs no rebuild.
- **A Rust crate compiled into teleprompt**, as the ones in this
  repository are. The same crate can also be built as a program of its
  own ([a Rust plugin as a program](#a-rust-plugin-as-a-program)).

`teleprompt plugins` lists the plugins installed as programs, asks each
what it is, and says why one cannot be used.

`teleprompt-plugin` has a module per contract, and the two kinds use
different ones:

| module | kind | what you implement |
|---|---|---|
| `scene` | scene | `SceneCompiler`: a block in your tool's own language, validated and split into shots, offline |
| `capture` | scene | `CaptureBackend`: a session of shots run, and a clip kept for each |
| `record` | scene | `Recorder`, optional: an author's working session recorded, for `teleprompt record` |
| `voice` | voice | `VoiceBackend`: a line of text turned into audio |
| `tool` | both | `Tool`, for what your plugin needs that teleprompt does not ship, and helpers to run it |
| `protocol` | both | a plugin as a program: teleprompt's side, and serving a Rust plugin |

A scene plugin hands teleprompt a `teleprompt_plugin::ScenePlugin`; a voice
plugin, a `teleprompt_plugin::VoicePlugin`.

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
declaring your own. A server the author runs, or a service with a key, is
`Found::Unknowable` with a `guide` saying how to set it up.

## Registering it

**As a program:** put it on PATH, or in the plugins directory
(`$TELEPROMPT_PLUGINS`, or `teleprompt/plugins` in your data directory,
such as `~/.local/share/teleprompt/plugins`). Install it however suits its
language: `pipx`, `npm install -g`, `cargo install`, or a copied file. A
built-in plugin's name wins over an installed one's, which `teleprompt
plugins` says. Publish it with the GitHub topic `teleprompt-plugin`.

**Compiled in:** add your crate to the workspace, and one line to
`teleprompt-cli`: `built_in()` in `src/scene.rs` for a scene plugin,
`plugins()` in `src/voice.rs` for a voice. Then add your crate to
`tools/check_deps.py`, with `PLUGIN` as what it may depend on.

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

Every plugin answers **`describe`** first, with what the two kinds share
and, beside it, what its kind says of itself:

```json
{"protocol": 1, "kind": "scene", "name": "card", "needs": [],
 "continues": false, "retimes": true}
```

Its `name` matches its program's. A tool in `needs` is `{"name", "what",
"license", "home", "program" or "package", "install":{"brew": "...", "apt":
"...", ...}}`, which `teleprompt setup` lists and installs. The rest of
the methods are the kind's own: [a scene plugin's](scene-plugins.md#the-protocol),
[a voice's](voices.md#as-a-program).

## A Rust plugin as a program

`teleprompt_plugin::protocol::serve` serves a `ScenePlugin` or a
`VoicePlugin` over stdin and stdout, so a crate written against the
contracts can be built as a program too:

```rust
// src/bin/teleprompt-scene-mine.rs
fn main() -> std::io::Result<()> {
    teleprompt_plugin::protocol::serve::scene(my_plugin::plugin())
}
```

`serve::voice(my_voice::plugin())` does the same for a voice.
