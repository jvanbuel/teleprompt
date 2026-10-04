# Example plugins

Two plugins as programs of their own, in Python with nothing beyond its
standard library, to show what any language can do
([extending teleprompt](../../docs/guide/plugins.md)):

- `teleprompt-scene-card`: a scene of plain colour cards (`color`,
  `hold`, `mark`), captured with ffmpeg. The smallest complete scene plugin.
- `teleprompt-voice-espeak`: narration spoken by eSpeak NG, offline, in
  over a hundred languages.

To use them, put this directory on PATH, or copy them into the plugins
directory, and check that teleprompt finds them:

    export PATH="$PWD/examples/plugins:$PATH"
    teleprompt plugins

Then a block can say `scene=card`, and `teleprompt.toml` can say
`[voice] backend = "espeak"`. The tests in `crates/teleprompt-plugin`
and `crates/teleprompt-cli` run them, so they stay in step with the
protocol, and `examples/plugin-authors` is a video made with both.
