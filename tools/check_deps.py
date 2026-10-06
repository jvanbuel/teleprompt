#!/usr/bin/env python3
"""Fail if a workspace crate depends on one it should not.

    python3 tools/check_deps.py

The layering is docs/design.md#crates: core at the bottom; the plugin
contracts, schedule and manifest on core alone and not on one another;
compile where they meet; render reading the manifest, not the compiler;
every scene plugin on the plugin contracts alone, every voice on the
voice contract alone; and only the CLI knowing every plugin and voice by
name. A new dependency between workspace
crates has to be added here, which is where the argument for it belongs.
Dev-dependencies are not checked.

The plugin crate holds every contract, capturing included, so the crates
that plan a video are also kept from its capture, protocol, record and
tool modules by name: nothing that plans a video can run a tool.
"""

import json
import pathlib
import re
import subprocess
import sys

# A plugin depends on the contracts it implements, and nothing else of
# teleprompt's: what an outside plugin can do, the built-in ones do.
PLUGIN = {"core", "plugin"}
# The voices teleprompt ships depend on the voice contract, and nothing else.
VOICE = {"core", "voice"}

ALLOWED = {
    "core": set(),
    "schedule": {"core"},
    "plugin": {"core"},
    "voice": {"core"},
    "manifest": {"core"},
    "voices": VOICE,
    "compile": {"core", "plugin", "schedule", "voice", "manifest"},
    "render": {"core", "manifest"},
    "vhs": PLUGIN,
    "asciinema": PLUGIN,
    "playwright": PLUGIN,
    "remotion": PLUGIN,
    "slidev": PLUGIN,
    "media": PLUGIN,
    # The desktop scene plugins, x11 and macos: one language and runner.
    "desktop": PLUGIN,
    "translate": {"core"},
    "listen": set(),
    # Ids and speaker names are the script's; who speaks when is what a
    # recognizer hears.
    "derive": {"core", "listen"},
    "listen-sherpa": {"listen"},
    # The protocol and the text; the compile reaches it through a trait
    # the engine implements.
    "lsp": {"core"},
    "testkit": set(),  # dev-dependency only; never a normal dependency
    # The engine, where every scene plugin and voice is composed: may
    # depend on anything.
    "teleprompt": None,
    # The command line over the engine, and the types its flags name.
    "cli": {"teleprompt", "core", "listen-sherpa"},
}

PREFIX = "teleprompt-"

# What plans a video, and so reads a scene's shots without running its tool.
PLANS = {"schedule", "manifest", "voice", "compile"}
RUNS_TOOLS = re.compile(r"teleprompt_plugin::(capture|protocol|record|tool)\b")


def runs_tools(crate):
    """The sources of `crate` that reach a module that runs tools."""
    src = pathlib.Path("crates") / f"{PREFIX}{crate}" / "src"
    return [
        str(p) for p in sorted(src.rglob("*.rs")) if RUNS_TOOLS.search(p.read_text())
    ]


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
    for crate in sorted(PLANS):
        for path in runs_tools(crate):
            problems.append(f"{path} uses the plugin crate's capture, protocol, record or tool module")
    for p in problems:
        print(f"error: {p}", file=sys.stderr)
    if problems:
        print("see docs/design.md#crates", file=sys.stderr)
        return 1
    print(f"crate dependencies ok ({len(ALLOWED)} crates)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
