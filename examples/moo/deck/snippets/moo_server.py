#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["fastapi>=0.110", "uvicorn>=0.29", "numpy>=1.26"]
# ///
"""A speech server that says every word as a moo.

It speaks OpenAI's speech API, so teleprompt needs no plugin for it:
`[backends.moo]` and its address are enough. The moo is a
real cow (moo.wav beside this file, CC0: see moo.wav.txt). Run it with
`uv run moo_server.py [port]`, which installs what it needs.
"""

import re
import sys
import wave
from pathlib import Path
from typing import Literal

import numpy as np
import uvicorn
from fastapi import FastAPI, HTTPException, Response
from numpy.typing import NDArray
from pydantic import BaseModel

RATE = 24_000  # what "pcm" means in the API: 16-bit mono at 24 kHz
Audio = NDArray[np.float64]  # mono samples at RATE, from -1 to 1


def load(path: Path) -> Audio:
    with wave.open(str(path)) as f:
        assert f.getframerate() == RATE and f.getnchannels() == 1
        samples = np.frombuffer(f.readframes(f.getnframes()), dtype="<i2")
    return samples / 32768


MOO = load(Path(__file__).with_name("moo.wav"))


# region voices
# Each voice is the same cow, played faster or slower: higher or lower.
VOICES: dict[str, float] = {"cow": 1.0, "calf": 1.5, "bull": 0.75}
# endregion


# region moo
def pitched(audio: Audio, factor: float) -> Audio:
    """`audio` played `factor` times as fast, so that much higher."""
    at = np.arange(0, len(audio) - 1, factor)
    faster: Audio = np.interp(at, np.arange(len(audio)), audio)
    return faster


def moo(seconds: float, voice: float) -> Audio:
    """The cow's moo, cut to `seconds` and faded out where it is cut."""
    cut = pitched(MOO, voice)[: int(seconds * RATE)]
    fade = min(len(cut), int(0.08 * RATE))
    cut[len(cut) - fade :] *= np.linspace(1, 0, fade)
    return cut


def silence(seconds: float) -> Audio:
    return np.zeros(int(seconds * RATE))


# endregion


# region speak
def speak(text: str, voice: float, speed: float) -> Audio:
    """Every word a moo as long as the word; punctuation, a pause."""
    said: list[Audio] = []
    for word, mark in re.findall(r"([\w']+)([.,!?;:]*)", text):
        said.append(moo((0.25 + 0.05 * len(word)) / speed, voice))
        pause = 0.4 if mark[:1] in ".!?" else 0.2 if mark else 0.06
        said.append(silence(pause / speed))
    return np.concatenate(said or [silence(0.2)])


# endregion


# region request
class SpeechRequest(BaseModel):
    """OpenAI's speech request, as much of it as a cow needs."""

    model: str
    input: str
    voice: str = "cow"
    speed: float = 1.0
    response_format: Literal["pcm"] = "pcm"


# endregion


app = FastAPI(title="moo")


# region answer
@app.post("/v1/audio/speech")
def speech(request: SpeechRequest) -> Response:
    if request.voice not in VOICES:
        raise HTTPException(400, f"no voice {request.voice!r}; try {', '.join(VOICES)}")
    audio = speak(request.input, VOICES[request.voice], request.speed)
    pcm = (np.clip(audio, -1, 1) * 32767).astype("<i2").tobytes()
    return Response(pcm, media_type="audio/pcm")


@app.get("/v1/audio/voices")
def voices() -> dict[str, list[str]]:
    return {"voices": list(VOICES)}


# endregion


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8890
    uvicorn.run(app, host="127.0.0.1", port=port, log_level="warning")
