# Voices

`voice.backend` chooses what speaks the narration. It defaults to `null`,
which produces silence of the estimated length and needs nothing
installed. `kokoro` talks HTTP to a
[Kokoro-FastAPI](https://github.com/remsky/Kokoro-FastAPI) server you run
yourself, in Docker or with pip, and `voicebox` to a
[Voicebox](https://voicebox.sh) app, which can speak in [your own
voice](#your-own-voice-voicebox). `gemini` is the one that isn't local:
[Google's Gemini TTS](#gemini-tts), with your own API key. teleprompt
bundles no model and runs no Python.

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

## Your own voice: Voicebox

[Voicebox](https://github.com/jamiepine/voicebox) is a free, local voice
studio (MIT) that clones a voice from a few recordings. teleprompt talks
to it over HTTP; you install and run it (`teleprompt setup voicebox`).
Record some lines in the prompter, then make a voice from those takes:

```bash
teleprompt voice clone Narrator
```

It sends your longest takes, up to six of 1.5 s or more, each with the
words it says (cloning is faithful only with the transcript, which a take
has), and prints the settings that use the voice:

```toml
[voice]
backend = "voicebox"
voice = "Narrator"     # a Voicebox voice, by name or id

[backends.voicebox]
base_url = "http://127.0.0.1:17493"
```

Now the lines you recorded play as recorded, and the rest are spoken in a
voice made from them, so the video sounds like one speaker. **Clone only
your own voice, or one you have permission to use.**

| setting | default | meaning |
|---|---|---|
| `base_url` | `http://127.0.0.1:17493` | the server; teleprompt doesn't start or manage it |
| `timeout_ms` | `300000` | limit for each line; cloned voices on large models are slow |
| `concurrency` | `1` | lines `dub` sends at once |
| `engine` | `qwen` | Voicebox's engine: `qwen` (cloned and designed voices), `chatterbox`, `luxtts`, `kokoro`, … |
| `model_size` | the server's | the engine's size, such as `1.7B` or `0.6B` |
| `seed` | `0` | sent with every line, so a line sounds the same each time; the engine, size and seed are in the cache key |

How a line is delivered can be asked for in words, for the engines that
take instructions (Qwen's preset and designed voices): `voice.instruct`
in `[voice]` or front matter for every line, or on one line,
`{#intro voice.instruct="warmly, with a smile"}`. It's part of the cache
key. Voicebox has no speed setting, so `voice.speed` must stay 1.0; ask
for "a little slower" instead.

Two things teleprompt never does with Voicebox: it never asks for its
"personality" rewriting, which would change your words (and with them the
captions, cues and line identity), and it never falls back to another
voice when the server is down. The models' licenses differ: Qwen3-TTS,
LuxTTS and Kokoro are Apache-2.0, Chatterbox MIT (and watermarks its
audio), and TADA's weights come under the Llama 3.2 Community License,
which has conditions. A voice cloned again under the same name keeps
speaking the old audio from the cache; give the new one a new name.

## Gemini TTS

`gemini` speaks with Google's Gemini 3.8 TTS models, which run at Google:
the narration is sent there, and your key pays for it. Make a key at
[aistudio.google.com/apikey](https://aistudio.google.com/apikey) and put
it in `GEMINI_API_KEY`. It's read only when a line is spoken, so `check`,
`plan` and `diff` need none.

```toml
[voice]
backend = "gemini"
voice = "Charon"       # one of the 30 prebuilt voices, or a custom voice's id
instruct = "a warm, unhurried documentary narrator"

[backends.gemini]
model = "gemini-3.8-flash-lite-tts"   # the cheaper one, for many lines
```

| setting | default | meaning |
|---|---|---|
| `model` | `gemini-3.8-flash-tts` | the model; `gemini-3.8-flash-lite-tts` costs less |
| `api_key_env` | `GEMINI_API_KEY` | the environment variable holding the key |
| `timeout_ms` | `120000` | limit for each line |
| `concurrency` | `4` | lines `dub` sends at once; lower it if the key's quota refuses them (429) |
| `seed` | none | sent with every line when set; the model and seed are in the cache key |

`voice.voice` defaults to `Kore`. The prebuilt voices include Zephyr, Puck,
Charon, Kore, Fenrir, Leda, Orus, Aoede, Enceladus and Schedar; AI Studio
plays all thirty. A voice you designed or replicated with Google's Voices
API is named by its id (`voice_…`); a stored one expires after seven
days, but the lines it spoke stay in the cache.

The words are spoken exactly as written, so direction never goes in the
line. `voice.instruct` is sent beside it as how to say it: style, pace,
accent. Gemini has no speed setting, so `voice.speed` must stay 1.0; ask
for "a little slower" instead. Its audio carries Google's SynthID
watermark, and its use falls under the
[Gemini API terms](https://ai.google.dev/gemini-api/terms).

## Testing against a real server

The Kokoro tests run against an in-process stub. One test uses a real
server and is ignored by default. Start a server on `localhost:8880`, then:

```bash
cargo test -p teleprompt-voice-kokoro --test real -- --ignored
```
