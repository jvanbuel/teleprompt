# An example scene plugin

`teleprompt-scene-card` is a scene plugin as a program of its own, in
Python with nothing beyond its standard library, to show what any language
can do ([writing a scene plugin](../../docs/guide/scene-plugins.md)). It
draws a scene of plain colour cards (`color`, `hold`, `mark`), captured
with ffmpeg: the smallest complete scene plugin. It type-checks with
`mypy --strict` and is formatted with ruff (`../ruff.toml`).

To use it, put this directory on PATH, or copy it into the plugins
directory, and check that teleprompt finds it:

    export PATH="$PWD/examples/plugins:$PATH"
    teleprompt setup card

Then a block can say `scene=card`. The tests in `crates/teleprompt-plugin`
and `crates/teleprompt-cli` run it, so it stays in step with the protocol,
and `examples/plugin-authors` is a video made with it.

A voice is not a plugin but a speech server: `../voices/espeak_server.py`
is one ([writing a voice](../../docs/guide/voices.md#writing-a-voice)).
