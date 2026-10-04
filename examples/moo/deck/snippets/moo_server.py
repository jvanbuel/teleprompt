#!/usr/bin/env python3
"""A speech server that says every word as a moo.

It speaks OpenAI's speech API, so teleprompt needs no plugin for it:
`[backends.moo] api = "openai"` and its address are enough. Python's
standard library alone; run it with `python3 moo_server.py [port]`.
"""
import json
import math
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

RATE = 24_000  # what "pcm" means in the API: 16-bit mono at 24 kHz

# region voices
# Each voice is a pitch that falls as the moo goes on, in hertz.
VOICES = {
    "cow": (150, 105),
    "calf": (300, 230),
    "bull": (95, 70),
}
# endregion


# region moo
def moo(seconds, start_hz, end_hz):
    """One moo: a hum that opens into "oo" and sinks as it fades."""
    n = int(seconds * RATE)
    out, phase = [], 0.0
    for i in range(n):
        t = i / n
        hz = start_hz + (end_hz - start_hz) * t
        hz *= 1 + 0.012 * math.sin(2 * math.pi * 5 * i / RATE)  # a wobble
        phase += 2 * math.pi * hz / RATE
        # "oo" is strong low harmonics; the closed "mm" is just the first.
        opened = min(1.0, t * 6)
        wave = (math.sin(phase)
                + opened * (0.6 * math.sin(2 * phase)
                            + 0.25 * math.sin(3 * phase)))
        envelope = min(1.0, t * 12) * min(1.0, (1 - t) * 5)
        out.append(wave * envelope * 0.35)
    return out
# endregion


# region speak
def speak(text, voice, speed):
    """Every word a moo as long as the word; punctuation, a pause."""
    start_hz, end_hz = VOICES[voice]
    samples = []
    for word, mark in re.findall(r"([\w']+)([.,!?;:]*)", text):
        seconds = (0.22 + 0.045 * len(word)) / speed
        samples += moo(seconds, start_hz, end_hz)
        pause = 0.4 if mark[:1] in ".!?" else 0.2 if mark else 0.07
        samples += [0.0] * int(pause / speed * RATE)
    return b"".join(
        int(max(-1, min(1, s)) * 32767).to_bytes(2, "little", signed=True)
        for s in samples
    )
# endregion


class Handler(BaseHTTPRequestHandler):
    # region request
    def do_POST(self):
        if self.path != "/v1/audio/speech":
            return self.answer(404, {"error": "no such path"})
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        voice = body.get("voice", "cow")
        if voice not in VOICES:
            return self.answer(400, {"error": f"no voice {voice!r}"})
        if body.get("response_format", "pcm") != "pcm":
            return self.answer(400, {"error": "this server speaks pcm only"})
        audio = speak(body["input"], voice, float(body.get("speed", 1.0)))
        self.send_response(200)
        self.send_header("Content-Type", "audio/pcm")
        self.send_header("Content-Length", str(len(audio)))
        self.end_headers()
        self.wfile.write(audio)
    # endregion

    # region list
    def do_GET(self):
        if self.path == "/v1/audio/voices":
            return self.answer(200, {"voices": list(VOICES)})
        if self.path == "/v1/models":
            return self.answer(200, {"data": [{"id": "moo-1"}]})
        self.answer(404, {"error": "no such path"})
    # endregion

    def answer(self, status, value):
        data = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8890
    print(f"mooing on http://localhost:{port}/v1", file=sys.stderr)
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
