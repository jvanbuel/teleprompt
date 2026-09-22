# Flowrs demo

A two-scene walkthrough: the same Airflow log, read once through the web
interface and once through [flowrs](https://github.com/jvanbuel/flowrs), a
terminal UI for Airflow.

It exists because it exercises both capture backends against real software
that nobody here controls — a browser driven by Playwright and a terminal
driven by VHS — in one script, with narration timed over both.

```
scripts/demo.md   the script: five paragraphs, five shots, two scenes
teleprompt.toml   project settings
tools/            an espeak-ng voice server, so the demo can be heard offline
```

## What it needs

| | |
|---|---|
| Airflow | reachable at `http://localhost:8080`, user `airflow`, password `airflow` |
| `flowrs` | on `PATH`, configured against that Airflow |
| `node` + `playwright` | `npm install` in this directory |
| `vhs`, `ttyd`, `ffmpeg` | on `PATH` |

`teleprompt doctor` reports on the last row. The first two are this demo's
own dependencies and it does not yet check them; if Airflow is down the
browser scene records a connection error and the terminal scene records an
empty DAG list, which is a confusing way to find out.

## Bringing up Airflow

The compose file is the stock Apache one, vendored in the flowrs repo —
use it from there rather than copying it here, so it stays in step with
the Airflow version flowrs is tested against.

```sh
git clone https://github.com/jvanbuel/flowrs
cd flowrs
echo "AIRFLOW_UID=$(id -u)" > .env
docker compose up -d          # apache/airflow:2.10.2, ~6 containers
```

Wait for `docker compose ps` to show every service healthy. First start
initializes the database and takes a few minutes.

The script reads `example_bash_operator`, which ships with Airflow but is
paused and has never run. Give it something to show:

```sh
curl -u airflow:airflow -X PATCH \
  "http://localhost:8080/api/v1/dags/example_bash_operator?update_mask=is_paused" \
  -H 'Content-Type: application/json' -d '{"is_paused": false}'
curl -u airflow:airflow -X POST \
  "http://localhost:8080/api/v1/dags/example_bash_operator/dagRuns" \
  -H 'Content-Type: application/json' -d '{"conf": {}}'
```

The run takes about fifteen seconds. Until it finishes, both scenes show a
DAG with no runs to open, and the walkthrough has nothing to walk through.

## Pointing flowrs at it

```sh
flowrs config add          # name it, endpoint http://127.0.0.1:8080, basic auth
```

The script assumes the server it opens on is the local one.

## Building

```sh
npm install
teleprompt build scripts/demo.md --out build/demo.mp4
```

Roughly a minute, most of it waiting on the browser. Expect
`5 line(s), 5 item(s), 0 slate(s)`; a slate means a shot recorded nothing,
and the count is the thing to watch.

## Hearing it

`teleprompt.toml` ships with `backend = "null"`, which renders silence of
the estimated length — the timing is real and there is nothing to hear.

For a voice with no model server, no weights and no network:

```sh
python3 tools/espeak-speech-server.py &     # needs espeak-ng and ffmpeg
```

then in `teleprompt.toml`:

```toml
[voice]
backend = "kokoro"

[backends.kokoro]
model = "espeak-ng-1.51-150wpm"
```

It sounds like formant synthesis, because it is. For a real voice, run
[Kokoro-FastAPI](https://github.com/remsky/Kokoro-FastAPI) on `:8880`
instead and set `model` back to `kokoro` — same endpoint, same config,
better voice.

Rename `model` whenever the voice behind it changes. It is the whole cache
key for synthesized audio, so leaving it alone gets you the old take.

## Notes on the two scenes

**The browser scene is a Playwright script.** Not a dialect that resembles
one — what is in the block is what runs, and anything Playwright can do is
available. Playwright cannot say in advance how long `page.click()` takes,
so the scene reports `Unknown` and the scheduler gives each shot the length
of the sentence spoken over it. The script does its work inside that window
and the last frame holds if it finishes early. If it overruns, the clip is
cut at the window and the log line says so.

That is why the script is full of `waitForTimeout`: it is pacing for a
viewer, not waiting for the page.

**Chromium** comes from the `playwright` package. On a machine where that
binary does not work, point the scene at another one:

```yaml
scene:
  ui:
    adapter: playwright
    executable: /path/to/chrome
```

**The terminal scene is a VHS tape**, and the whole scene is one session:
shot *n* opens on the screen shot *n-1* left behind. That is what lets the
three flowrs shots be a single walkthrough cut into three clips rather than
three restarts of the program.
