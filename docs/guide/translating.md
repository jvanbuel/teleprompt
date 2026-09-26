# Translating a video

One script can become a video in several languages. The narration is
translated. The terminal sessions, browser scripts and slides are the
same in every language, and so is how a shot follows the words.

```bash
teleprompt translate scripts/tour.md --to nl
teleprompt build scripts/tour.md --locale nl
```

`translate` writes `scripts/tour.nl.yaml` beside the script. `build`, like
`plan`, `dub` and every other command, reads it when you pass
`--locale nl`, and writes `build/tour.nl.mp4` with Dutch narration and
Dutch captions.

## The translation file

```yaml
# The `nl` narration of tour.md, kept by `teleprompt translate`.
chapters:
  introduction:
    from: 3fa2c1d9e0b4
    text: Inleiding
lines:
  welcome:
    from: 9ab30c7e21f5
    text: Welkom bij Acme.
cues:
  start-a:
    from: 51c0de442a90
    text: config add
```

Commit it next to the script, and read it over before you build: every
line is spoken exactly as written. Edit any `text` you'd say differently.

`from` records which English the entry was translated from. When you edit
a line in the script, its translation still plays, but every command
reports it as out of date until you translate again. A line with no
translation at all is an error.

Lines are matched by id. Pin the ids of lines you'll translate with
`{#id}` (see [line identity](scripts.md)), because a derived id changes
when its chapter heading does, and its translation goes with it.

## Translating again

Run `translate` again after editing the script. It asks only for the
lines that are new or changed since, and sends the rest along so the
terminology stays the same. What you corrected by hand in the file stays
as you wrote it, unless its English changes.

## Cues

A block's `cue="…"` names words in the line above it, and the Dutch line
doesn't contain the English words. So `translate` also asks for the words
of the translation that say the same thing, and records them under
`cues:`. A cue that's a command, like `cue="config add"`, usually stays as
it is. If a cue has no translation, the shot starts as far into the
Dutch line as it was into the English one, and `plan` says so.

## Choosing a translator

By default `translate` uses Claude, through the Anthropic API. Set your
API key first:

```bash
export ANTHROPIC_API_KEY=...
teleprompt translate scripts/tour.md --to fr
teleprompt translate scripts/tour.md --to fr --model claude-sonnet-5
```

This sends the script's narration to Anthropic.

To use another service, or a model on your own machine, give a command
instead. It's run by the shell, gets the request as JSON on stdin, and
answers on stdout:

```bash
teleprompt translate scripts/tour.md --to de --command ./my-translator
```

```json
{
  "source": "en",
  "target": "de",
  "existing": [{"id": "line:welcome", "english": "Welcome to Acme.", "translation": "Willkommen bei Acme."}],
  "translate": [
    {"id": "line:start", "kind": "line", "english": "Run flowrs config add to start."},
    {"id": "cue:start-a", "kind": "cue", "english": "config add", "line": "line:start"}
  ]
}
```

```json
{"items": [{"id": "line:start", "text": "..."}, {"id": "cue:start-a", "text": "config add"}]}
```

Anything it doesn't answer is left for next time.

## A voice per language

Settings for one language go under `[locale.<code>]`, over the rest:

```toml
[voice]
backend = "kokoro"
voice = "af_heart"

[locale.fr.voice]
voice = "ff_siwis"
```

Front matter takes them too (`locale: { fr: { voice: { speed: 1.1 } } }`).
