#!/usr/bin/env python3
"""An OpenAI-compatible /v1/audio/speech endpoint backed by espeak-ng.

The kokoro backend speaks plain OpenAI audio: POST /v1/audio/speech with
`response_format: "pcm"`, answered with raw little-endian 16-bit mono at
24 kHz. Nothing in that contract says a neural model has to be behind it.
This serves the same contract from espeak-ng, so the demo has a voice
without a model server, weights, or a GPU.

It sounds like formant synthesis, because it is. It is here so the demo
can be heard offline, not because it is the voice you would ship.

    python3 tools/espeak-speech-server.py      # serves 127.0.0.1:8880

Set `backends.kokoro.model` to something that names this server, not
"kokoro": the model string is the entire cache key for synthesized audio,
so reusing the name hands you yesterday's take from a different voice.
"""
import json
import subprocess
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

RATE = 24_000
BASE_WPM = 150
VOICES = ["en-us", "en-gb", "en-us+f3", "en-gb+f2"]


def synth(text: str, voice: str, speed: float) -> bytes:
    wpm = max(80, min(450, round(BASE_WPM * speed)))
    espeak = subprocess.run(
        ["espeak-ng", "-v", voice, "-s", str(wpm), "--stdout", text],
        capture_output=True,
    )
    if espeak.returncode != 0:
        raise RuntimeError(espeak.stderr.decode(errors="replace").strip())
    ff = subprocess.run(
        ["ffmpeg", "-hide_banner", "-loglevel", "error", "-i", "pipe:0",
         "-f", "s16le", "-acodec", "pcm_s16le", "-ar", str(RATE), "-ac", "1",
         "pipe:1"],
        input=espeak.stdout, capture_output=True,
    )
    if ff.returncode != 0:
        raise RuntimeError(ff.stderr.decode(errors="replace").strip())
    return ff.stdout


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):  # keep the console readable
        pass

    def _send(self, code, body, ctype):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.rstrip("/") == "/v1/audio/voices":
            self._send(200, json.dumps({"voices": VOICES}).encode(),
                       "application/json")
        else:
            self._send(404, b"not found", "text/plain")

    def do_POST(self):
        if self.path.rstrip("/") != "/v1/audio/speech":
            self._send(404, b"not found", "text/plain")
            return
        n = int(self.headers.get("Content-Length", 0))
        try:
            req = json.loads(self.rfile.read(n) or b"{}")
        except json.JSONDecodeError as e:
            self._send(400, str(e).encode(), "text/plain")
            return
        text = (req.get("input") or "").strip()
        if not text:
            self._send(400, b"empty input", "text/plain")
            return
        voice = req.get("voice") or "en-us"
        if voice not in VOICES:
            voice = "en-us"
        try:
            pcm = synth(text, voice, float(req.get("speed") or 1.0))
        except RuntimeError as e:
            self._send(500, str(e).encode(), "text/plain")
            return
        self._send(200, pcm, "audio/pcm")


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", 8880), Handler).serve_forever()
