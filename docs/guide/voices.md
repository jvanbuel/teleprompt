# Voices

`voice.backend` chooses what speaks the narration. It defaults to `null`,
which produces silence of the estimated length and needs nothing
installed. Everything else is a speech server, and most of those speak
OpenAI's speech API, so one backend speaks to all of them:

- `kokoro`: a [Kokoro-FastAPI](https://github.com/remsky/Kokoro-FastAPI)
  server you run yourself, in Docker or with pip. Local, free, and the
  one the examples use.
- `openai`: [OpenAI's own](#openai), with your API key.
- [any other server](#any-openai-compatible-server) that speaks the API,
  under a name you give it in `teleprompt.toml`. No plugin needed.

Two servers speak APIs of their own: `voicebox`, a
[Voicebox](https://voicebox.sh) app, which can speak in [your own
voice](#your-own-voice-voicebox), and `gemini`, [Google's Gemini
TTS](#gemini-tts), with your own API key. And a voice that is not a server
at all, such as a program on your machine, is a
[plugin](#writing-a-voice): `examples/plugins` has one spoken by eSpeak
NG. teleprompt bundles no model and runs no Python.

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
| `base_url` | `http://localhost:8880` | the server, or its API root (`…/v1`); teleprompt doesn't start or manage it |
| `timeout_ms` | `30000` | limit for each synthesis request |
| `concurrency` | `4` | how many lines `dub` sends at once; a local server is the bottleneck, so more only slows it down |
| `model` | `kokoro` | sent as the request's `model`, and the cache key for everything this server produces |
| `voice` | none | the voice when `voice.voice` gives none |
| `api_key_env` | none | the environment variable holding a key, for a server that wants one |
| `sample_rate` | `24000` | the rate of the server's raw PCM, for a server that sends another |
| `word_timings` | `false` | ask for when each word is said (see [below](#word-timings)) |

`backends.*` is read only from `teleprompt.toml`. A script that repeats it
in front matter gets a warning.

## When the voice is needed

`check` and `plan` never synthesize, never read audio and never open
a socket. They read durations from the cache, which holds real measurements
once a line has been synthesized and a word-count estimate before that. So
the whole editing loop works on a laptop with no server running.

```
$ teleprompt plan scripts/tour.md --format json | grep duration_source
  "duration_source": "estimated"
```

`dub` does the real synthesis and fills the cache, after which the same
`plan` is still instant and says `measured`. A timeline committed from a
cold cache therefore drifts the first time you dub. `plan --check` reports that as
`now measured` rather than as an edit, and `plan` warns when it emits
estimates.

## The cache

Synthesized lines are kept in `.teleprompt/cache/voice/`, keyed on
everything that changes the audio: the backend, its `model`, the voice,
speed, locale and text. The server's address isn't part of the key, so a
cache made against one machine's server is valid against another's running
the same model. The flip side is that two servers answering to the same
`model` share a cache, so name the model after what is actually loaded.
`teleprompt setup` shows the model next to the address.

A corrupt entry is treated as a miss and replaced on the next dub.
`teleprompt cache` reports what the caches hold.

## When the server is down

An unreachable, slow or failing server fails `dub` with exit 1, naming the
URL and the line. It never substitutes silence. `teleprompt setup` reports
a down server as a warning, because `check` and `plan` don't need it.

## Word timings

With `word_timings = true`, Kokoro also returns when each word is said. The
manifest then publishes every word's `start_ms` and `end_ms`, and a `cue=`
starts exactly on its phrase. Kokoro-FastAPI serves timings from
`/dev/captioned_speech`, a development endpoint, which is why they're
opt-in. A server without that endpoint fails `dub` with a message naming
the setting. Turning timings on changes the cache key once, so each line is
synthesized again, this time with its timings.

## OpenAI

`openai` speaks with OpenAI's speech models, which run at OpenAI: the
narration is sent there, and your key pays for it. Put the key in
`OPENAI_API_KEY`. It's read only when a line is spoken, so `check` and
`plan` need none.

```toml
[voice]
backend = "openai"
voice = "coral"
instruct = "a calm, friendly narrator"

[backends.openai]
model = "gpt-4o-mini-tts"
```

Its settings are those above, with `base_url`
`https://api.openai.com/v1`, `api_key_env` `OPENAI_API_KEY`, `model`
`gpt-4o-mini-tts`, `voice` `alloy` and `timeout_ms` `60000`.
`voice.instruct` is sent as the request's `instructions`, which
`gpt-4o-mini-tts` takes and the older `tts-1` ignores. OpenAI lists no
voices, so `dub` does not check a script's against them.

## Any OpenAI-compatible server

A speech server that answers OpenAI's `POST /v1/audio/speech` is a voice
under any name you give it, with `api = "openai"` and its address:

```toml
[voice]
backend = "studio"
voice = "amy"

[backends.studio]
api = "openai"
base_url = "http://gpu-box:8000/v1"
model = "piper-amy"
```

The settings are the table's above; `base_url` has no default, `model`
defaults to `tts-1` and `concurrency` to 2. teleprompt asks for
`response_format: "pcm"`, which the API defines as 16-bit mono at
24 kHz: a server that sends another rate is given it as `sample_rate`. A
server that lists its voices at `/v1/audio/voices`, as Kokoro-FastAPI
does, has a script's checked against them before `dub` speaks anything.

Name the `model` after what the server actually has loaded. It is the
cache key, and the address is not, so a cache moves between machines
([the cache](#the-cache)).

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
it in `GEMINI_API_KEY`. It's read only when a line is spoken, so `check`
and `plan` need none. `teleprompt setup`, in the project, asks whether the key
opens the model, which costs no speech.

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

## Writing a voice

A voice that is a speech server is best written as one that speaks
OpenAI's API ([above](#any-openai-compatible-server)): then it needs
nothing of teleprompt's, and works with other tools too. Otherwise, a
voice is a plugin ([extending teleprompt](plugins.md) has how one ships
and is found).

### As a program

`teleprompt-voice-<name>` describes itself with `"kind": "voice"` and what
it can do beyond speaking a line: `word_timings`, `speed_control`,
`lists_voices` (it answers `voices`), `probes` (it answers `probe`) and
`address` (where its server is, if it has one). It answers:

| method | params | result |
|---|---|---|
| `configure` | `settings`, its `[backends.<name>]` table, or null | `version`: what the audio depends on beyond the request, such as a model. Part of the voice cache's key, so never a path or a host |
| `synthesize` | `text`, `locale`, `voice`, `speed`, `instruct`, `out` | the line written to `out` as a WAV file; `word_timings` if it has them |
| `voices` | | `voices`: the names its `voice.voice` may give |
| `probe` | | `line`: one line on its server, for `setup` |

`examples/plugins/teleprompt-voice-espeak` is one, in Python. Cloning a
voice is not in version 1 of the protocol; it is compiled-in only for now.

### As a Rust crate

A voice implements `VoiceBackend`: `id`, `capabilities`, and `synthesize`,
which returns PCM. The other methods have defaults. Override them for what
your server can do: `concurrency` (lines sent at once), `address` (where
the server is), `voices` (checked before a dub), `probe` (the line `setup`
prints) and `clone_voice`.

It is registered as a `VoicePlugin`: its id, which `voice.backend` names,
and how it is built from its own `[backends.<id>]` settings.

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
model. `crates/teleprompt-voice-openai` is the example, and the backend
behind `kokoro`, `openai` and every named server.

## Testing against a real server

The OpenAI-compatible backend's tests run against an in-process stub. One test uses a real
server and is ignored by default. Start a server on `localhost:8880`, then:

```bash
cargo test -p teleprompt-voice-openai --test real -- --ignored
```
