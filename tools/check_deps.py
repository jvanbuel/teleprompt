#!/usr/bin/env python3
"""Fail if a workspace crate depends on one it should not.

    python3 tools/check_deps.py

The layering is docs/design.md#crates: core at the bottom; the scene
contract and the manifest on core alone and not on one another;
the pipeline where they meet; render reading the manifest, not the compiler;
every scene plugin on the scene crate alone, every voice on the
voice contract alone; and only the CLI (its `registry` module) knowing every plugin and voice by
name. A new dependency between workspace
crates has to be added here, which is where the argument for it belongs.
Dev-dependencies are not checked.

The scene crate holds the contract a plugin implements and the code that
runs one; the compiler may use only its contract (`scene::contract`), which
review enforces.
"""

import json
import subprocess
import sys

# The built-in scene plugins depend on the contracts they implement, and
# nothing else of teleprompt's: what an outside plugin can do, they do.
PLUGIN = {"scene"}
# The voices teleprompt ships depend on the voice contract, and nothing else.
VOICE = {"core", "voice"}

ALLOWED = {
    "core": set(),
    # The script language: parsing, resolving, editing and translating.
    "script": {"core"},
    # What a scene plugin implements: the contract, capture, recording and
    # the protocol.
    "scene": {"core"},
    "voice": {"core"},
    "manifest": {"core"},
    "voices": VOICE,
    # A script to its timeline and manifest: the scheduler and the compiler.
    "pipeline": {"core", "script", "scene", "voice", "manifest"},
    "render": {"core", "manifest"},
    # The scene plugins teleprompt ships, a module each.
    "scenes": PLUGIN,
    "listen": set(),
    "testkit": set(),  # dev-dependency only; never a normal dependency
    # A project and what the commands do to its scripts: compiled, dubbed,
    # captured, built, edited and translated. It is handed its scene
    # plugins and voices as a `Registry` and knows none by name, so no
    # plugin crate and no voice is here.
    "project": {
        "core", "script", "scene", "voice", "manifest", "pipeline", "render",
    },
    # What `setup` finds and installs: what the plugins and voices need.
    "setup": {"core", "scene"},
    # The language server, with the real compile as its analyzer.
    "lsp": {"project", "core", "script"},
    # Drafts from what was said: `import` and `record`.
    "draft": {"project", "setup", "core", "script", "listen", "scene", "voice"},
    # The prompter: its page, API and session, following a reader by ear.
    "serve": {"project", "setup", "core", "script", "listen", "voice"},
    # The command line, composing the rest, and the commands that only
    # read or print: check, plan, new, cache, voice clone.
    "cli": {
        "project", "setup", "lsp", "draft", "serve", "core", "pipeline", "script",
        "voice", "scene", "scenes", "voices",
    },
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
