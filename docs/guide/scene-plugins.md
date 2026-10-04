# Writing a scene plugin

A scene plugin brings a kind of scene a script can show. It is one of the
two kinds of plugin ([extending teleprompt](plugins.md) has how one ships,
says what it needs, and is found); this page is what a scene plugin is.

A project configures a scene plugin into scenes: `[scene.demo] plugin =
"vhs"` with a theme and a size, and `[scene.wide]` with another. A block
names the scene, and the scene names its plugin.

## The contract

A scene plugin is a scene compiler, a capture backend and optionally a
recorder, handed over as one `teleprompt_plugin::ScenePlugin`, named by the
compiler's `kind()`. That name is what a scene's `plugin` gives.

```rust
pub fn plugin() -> teleprompt_plugin::ScenePlugin {
    teleprompt_plugin::ScenePlugin::new(MyScene, MyCapture::default())
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
sessions too. `examples/plugins/teleprompt-scene-card` is a complete
scene plugin in Python, as a program.

## Testing it

The `mock` scene plugin (`teleprompt_plugin::scene::MockScene` and
`capture::mock::MockCapture`) shows the contract with no tool behind it.
A scene compiler is tested without its tool. A capture backend is tested
against the real tool where it is installed, and skips, saying so, where
it is not.

## The protocol

As a program, `teleprompt-scene-<name>` describes itself with
`"kind": "scene"`, `continues` (whether a shot opens on the screen the
previous one left; true unless said) and `retimes` (whether it answers
`retime`), and answers:

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

Not in version 1: recording a session, files outside a block that a scene
reads, and `include=…#fragment`. They are compiled-in only for now.
