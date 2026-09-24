# Voices

`voice.backend` chooses what speaks the narration. It defaults to `null`,
which produces silence of the estimated length and needs nothing
installed. The other backend is `kokoro`, which talks HTTP to a
[Kokoro-FastAPI](https://github.com/remsky/Kokoro-FastAPI) server you run
yourself, in Docker or with pip. teleprompt bundles no model and runs no
Python.

```toml
# teleprompt.toml
[voice]
backend = "kokoro"
voice = "af_heart"
speed = 1.0

[backends.kokoro]
base_url = "http://localhost:8880"
timeout_ms = 30000
concurrency = 4
model = "kokoro"
word_timings = false
```

| setting | default | meaning |
|---|---|---|
| `base_url` | `http://localhost:8880` | the server; teleprompt doesn't start or manage it |
| `timeout_ms` | `30000` | limit for each synthesis request |
| `concurrency` | `4` | how many lines `dub` sends at once; a local server is the bottleneck, so more only slows it down |
| `model` | `kokoro` | sent as the request's `model`, and the cache key for everything this server produces |
| `word_timings` | `false` | ask for when each word is said (see [below](#word-timings)) |

`backends.*` is read only from `teleprompt.toml`. A script that repeats it
in front matter gets a warning.

## When the voice is needed

`check`, `plan` and `diff` never synthesize, never read audio and never open
a socket. They read durations from the cache, which holds real measurements
once a line has been synthesized and a word-count estimate before that. So
the whole editing loop works on a laptop with no server running.

```
$ teleprompt plan scripts/tour.md --format json | grep duration_source
  "duration_source": "estimated"
```

`dub` does the real synthesis and fills the cache, after which the same
`plan` is still instant and says `measured`. A timeline committed from a
cold cache therefore drifts the first time you dub. `diff` reports that as
`now measured` rather than as an edit, and `plan` warns when it emits
estimates.

## The cache

Synthesized lines are kept in `.teleprompt/cache/voice/`, keyed on
everything that changes the audio: the backend, its `model`, the voice,
speed, locale and text. The server's address isn't part of the key, so a
cache made against one machine's server is valid against another's running
the same model. The flip side is that two servers answering to the same
`model` share a cache, so name the model after what is actually loaded.
`doctor` shows the model next to the address.

A corrupt entry is treated as a miss and replaced on the next dub.
`teleprompt cache` reports what the caches hold.

## When the server is down

An unreachable, slow or failing server fails `dub` with exit 1, naming the
URL and the line. It never substitutes silence. `doctor` reports a down
server as a warning, because `check`, `plan` and `diff` don't need it.

## Word timings

With `word_timings = true`, Kokoro also returns when each word is said. The
manifest then publishes every word's `start_ms` and `end_ms`, and a `cue=`
starts exactly on its phrase. Kokoro-FastAPI serves timings from
`/dev/captioned_speech`, a development endpoint, which is why they're
opt-in. A server without that endpoint fails `dub` with a message naming
the setting. Turning timings on changes the cache key once, so each line is
synthesized again, this time with its timings.

## Voice tiers

`voice.source` asks for `synthetic` (the default), `cloned` or `recorded`.
Only `synthetic` has backends today, so the other two fall back to it. The
manifest records what was asked for, what was used and why, and
`dub --strict-voice` makes a fallback fail with exit 4.

## Testing against a real server

The Kokoro tests run against an in-process stub. One test uses a real
server and is ignored by default. Start a server on `localhost:8880`, then:

```bash
cargo test -p teleprompt-voice-kokoro --test real -- --ignored
```
