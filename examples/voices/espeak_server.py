#!/usr/bin/env python3
"""A speech server that speaks with eSpeak NG, in OpenAI's speech API.

eSpeak NG is a small, offline speech synthesizer: robotic, but free, local
and in over a hundred languages. Any server that answers
`POST /v1/audio/speech` as OpenAI's does is a voice for teleprompt, with no
plugin: name it in teleprompt.toml.

    [backends.espeak]
    base_url = "http://localhost:8891/v1"
    model = "espeak"

    [voice]
    backend = "espeak"
    voice = "en-us"        # `espeak-ng --voices` lists them

Run it with `python3 espeak_server.py [port] [--wpm N]`. Python's standard
library alone, and espeak-ng.
"""

from __future__ import annotations

import argparse
import array
import json
import shutil
import subprocess
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from io import BytesIO
from typing import Any

# What `response_format: "pcm"` is in OpenAI's API: 16-bit mono at 24 kHz.
RATE = 24_000


def espeak(*args: str) -> bytes:
    """Runs eSpeak NG and returns what it printed."""
    program = shutil.which("espeak-ng") or shutil.which("espeak")
    if program is None:
        raise RuntimeError("espeak-ng is not on PATH")
    return subprocess.run([program, *args], check=True, capture_output=True).stdout


# region speak
def resampled(samples: array.array[int], rate: int) -> array.array[int]:
    """`samples` at `rate`, as the same sound at RATE: linear, which is
    enough for a voice this plain."""
    out = array.array("h")
    last = len(samples) - 1
    for i in range(len(samples) * RATE // rate):
        at = i * rate / RATE
        j = int(at)
        out.append(round(samples[j] + (samples[min(j + 1, last)] - samples[j]) * (at - j)))
    return out


def speak(text: str, voice: str, speed: float, wpm: int) -> bytes:
    """`text` said by eSpeak NG, as 16-bit little-endian PCM at RATE."""
    wav = espeak("-v", voice, "-s", str(round(wpm * speed)), "--stdout", text)
    with wave.open(BytesIO(wav)) as f:
        # Its header promises more frames than there are: read what is there.
        rate, frames = f.getframerate(), f.readframes(f.getnframes())
    return resampled(array.array("h", frames), rate).tobytes()


# endregion


# region answer
class Speech(BaseHTTPRequestHandler):
    """OpenAI's speech API, as much of it as a narrator needs."""

    wpm = 160

    def do_POST(self) -> None:
        if self.path != "/v1/audio/speech":
            return self.reply(404, {"error": f"no {self.path}"})
        request: dict[str, Any] = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if request.get("response_format", "pcm") != "pcm":
            return self.reply(400, {"error": "this server answers in pcm only"})
        try:
            pcm = speak(
                request["input"], request.get("voice") or "en-us", request.get("speed", 1.0), self.wpm
            )
        except subprocess.CalledProcessError as e:
            return self.reply(400, {"error": e.stderr.decode().strip()})
        self.send_response(200)
        self.send_header("Content-Type", "audio/pcm")
        self.send_header("Content-Length", str(len(pcm)))
        self.end_headers()
        self.wfile.write(pcm)

    def do_GET(self) -> None:
        if self.path != "/v1/audio/voices":
            return self.reply(404, {"error": f"no {self.path}"})
        # Pty Language Age/Gender VoiceName File Other: the language column.
        rows = espeak("--voices").decode().splitlines()[1:]
        self.reply(200, {"voices": sorted({row.split()[1] for row in rows if row.split()})})

    def reply(self, status: int, body: dict[str, Any]) -> None:
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


# endregion


if __name__ == "__main__":
    args = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    args.add_argument("port", type=int, nargs="?", default=8891)
    args.add_argument("--wpm", type=int, default=160, help="words per minute at speed 1.0")
    given = args.parse_args()
    Speech.wpm = given.wpm
    ThreadingHTTPServer(("127.0.0.1", given.port), Speech).serve_forever()
