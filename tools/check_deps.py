#!/usr/bin/env python3
"""Fail if a workspace crate depends on one it should not.

    python3 tools/check_deps.py

The layering is docs/design.md#crates: core at the bottom; schedule,
scene, voice, capture and manifest on core alone and not on one another;
compile where they meet; render reading the manifest, not the compiler;
adapters on the scene and capture contracts; and only the CLI knowing
every adapter and backend by name. A new dependency between workspace
crates has to be added here, which is where the argument for it belongs.
Dev-dependencies are not checked.
"""

import json
import subprocess
import sys

ADAPTER = {"core", "scene", "capture"}

ALLOWED = {
    "core": set(),
    "schedule": {"core"},
    "scene": {"core"},
    "voice": {"core"},
    "capture": {"core"},
    "manifest": {"core"},
    "cache": {"core", "voice"},
    "voice-null": {"core", "voice"},
    "voice-kokoro": {"core", "voice"},
    "voice-voicebox": {"voice"},
    "voice-gemini": {"voice"},
    "compile": {"core", "scene", "schedule", "voice", "cache", "manifest"},
    "render": {"core", "manifest"},
    "vhs": ADAPTER,
    "asciinema": ADAPTER,
    "playwright": ADAPTER,
    "remotion": ADAPTER,
    "slidev": ADAPTER,
    "media": ADAPTER,
    # What the desktop adapters share: their language and runner. An
    # adapter in all but name, registered by the platforms' plugins.
    "desktop": ADAPTER,
    "x11": ADAPTER | {"desktop"},
    "macos": ADAPTER | {"desktop"},
    "translate": {"core"},
    "listen": set(),
    "derive": set(),
    "listen-sherpa": {"listen"},
    "prompter": {"core", "compile", "listen", "voice"},
    # The protocol and the text; the compile reaches it through a trait
    # the CLI implements.
    "lsp": {"core"},
    "testkit": set(),  # dev-dependency only; never a normal dependency
    "cli": None,  # the composition root: may depend on anything
}

PREFIX = "teleprompt-"


def short(name):
    return name[len(PREFIX):] if name.startswith(PREFIX) else name


def main():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    problems = []
    for package in json.loads(out)["packages"]:
        crate = short(package["name"])
        if crate not in ALLOWED:
            problems.append(f"{crate}: not in tools/check_deps.py; add its allowed dependencies")
            continue
        allowed = ALLOWED[crate]
        if allowed is None:
            continue
        for dep in package["dependencies"]:
            if dep["kind"] is not None or not dep["name"].startswith(PREFIX):
                continue
            if short(dep["name"]) not in allowed:
                problems.append(f"{crate} depends on {short(dep['name'])}, which it may not")
    for p in problems:
        print(f"error: {p}", file=sys.stderr)
    if problems:
        print("see docs/design.md#crates", file=sys.stderr)
        return 1
    print(f"crate dependencies ok ({len(ALLOWED)} crates)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
