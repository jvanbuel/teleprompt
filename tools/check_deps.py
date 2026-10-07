#!/usr/bin/env python3
"""Fail if a workspace crate depends on one it should not.

    python3 tools/check_deps.py

The layering is docs/design.md#crates: core at the bottom; the scene
contract and the manifest on core alone and not on one another;
the pipeline where they meet; render reading the manifest, not the compiler;
every scene plugin on the scene crate alone, every voice on the
voice contract alone; and only the CLI knowing every plugin and voice by
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

# A plugin depends on the contracts it implements, and nothing else of
# teleprompt's: what an outside plugin can do, the built-in ones do.
PLUGIN = {"scene"}
# The voices teleprompt ships depend on the voice contract, and nothing else.
VOICE = {"core", "voice"}

PLUGIN_CRATES = {"vhs", "asciinema", "playwright", "remotion", "slidev", "media", "desktop"}

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
    "vhs": PLUGIN,
    "asciinema": PLUGIN,
    "playwright": PLUGIN,
    "remotion": PLUGIN,
    "slidev": PLUGIN,
    "media": PLUGIN,
    # The desktop scene plugins, x11 and macos: one language and runner.
    "desktop": PLUGIN,
    "listen": set(),
    "testkit": set(),  # dev-dependency only; never a normal dependency
    # A project and what the commands do to its scripts: compiled, dubbed,
    # captured, built, edited and translated. It is handed its scene
    # plugins and voices as a `Registry` and knows none by name, so no
    # plugin crate and no voice is here.
    "project": {
        "core", "script", "scene", "voice", "manifest", "pipeline", "render",
        "setup",
    },
    # The one crate that knows every scene plugin and voice by name.
    "registry": {"project", "scene", "voices"} | PLUGIN_CRATES,
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
        "project", "registry", "setup", "lsp", "draft", "serve", "core", "pipeline",
        "script", "voice",
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
