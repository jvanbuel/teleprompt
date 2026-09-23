#!/usr/bin/env python3
"""Size and shape of each crate, so a cleanup can show what it changed.

    python3 tools/metrics.py          # a Markdown table
    python3 tools/metrics.py --json   # the same numbers, for comparing runs

Counts only `src/`, except for tests, which counts `#[test]`-style
attributes across the whole crate. Lines are non-blank; a comment line is one
that starts with `//`. Function length is measured from the `fn` line to its
closing brace by brace matching, which string literals containing braces can
throw off by a little; the numbers are for trends, not for proofs.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LONG_FN = 80

FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:const\s+)?(?:unsafe\s+)?fn\s+(\w+)")
PUB_ITEM = re.compile(
    r"^\s*pub\s+(?:async\s+)?(?:fn|struct|enum|trait|type|const|static|mod|use)\b"
)
TEST_ATTR = re.compile(r"^\s*#\[(?:tokio::)?test\b")


def functions(lines):
    """(name, length) of every fn with a body."""
    out = []
    for i, line in enumerate(lines):
        m = FN.match(line)
        if not m:
            continue
        depth, opened = 0, False
        for j in range(i, len(lines)):
            code = lines[j].split("//", 1)[0]
            if not opened and ";" in code and "{" not in code:
                break  # a declaration without a body
            depth += code.count("{") - code.count("}")
            opened = opened or "{" in code
            if opened and depth <= 0:
                out.append((m.group(1), j - i + 1))
                break
    return out


def crate_metrics(crate):
    code = comments = pub_items = 0
    fns = []
    for path in sorted((crate / "src").rglob("*.rs")):
        lines = path.read_text().splitlines()
        for line in lines:
            s = line.strip()
            if not s:
                continue
            if s.startswith("//"):
                comments += 1
            else:
                code += 1
            if PUB_ITEM.match(line):
                pub_items += 1
        rel = path.relative_to(crate / "src")
        fns += [(f"{rel}::{name}", n) for name, n in functions(lines)]
    tests = sum(
        1
        for path in crate.rglob("*.rs")
        for line in path.read_text().splitlines()
        if TEST_ATTR.match(line)
    )
    longest = max(fns, key=lambda f: f[1], default=("-", 0))
    return {
        "code": code,
        "comments": comments,
        "comment_ratio": round(comments / code, 2) if code else 0.0,
        "longest_fn": longest[0],
        "longest_fn_lines": longest[1],
        "fns_over_limit": sum(1 for _, n in fns if n > LONG_FN),
        "pub_items": pub_items,
        "tests": tests,
    }


def main():
    crates = sorted(p for p in (ROOT / "crates").iterdir() if (p / "Cargo.toml").exists())
    report = {c.name: crate_metrics(c) for c in crates}
    total = {
        k: sum(m[k] for m in report.values())
        for k in ("code", "comments", "fns_over_limit", "pub_items", "tests")
    }
    total["comment_ratio"] = round(total["comments"] / total["code"], 2)
    total["longest_fn_lines"] = max(m["longest_fn_lines"] for m in report.values())

    if "--json" in sys.argv:
        print(json.dumps({"crates": report, "total": total}, indent=2))
        return

    print(f"| crate | code | comments | ratio | longest fn | fns > {LONG_FN} | pub items | tests |")
    print("|---|--:|--:|--:|---|--:|--:|--:|")
    for name, m in report.items():
        print(
            f"| {name} | {m['code']} | {m['comments']} | {m['comment_ratio']} "
            f"| {m['longest_fn']} ({m['longest_fn_lines']}) | {m['fns_over_limit']} "
            f"| {m['pub_items']} | {m['tests']} |"
        )
    print(
        f"| **total** | {total['code']} | {total['comments']} | {total['comment_ratio']} "
        f"| ({total['longest_fn_lines']}) | {total['fns_over_limit']} "
        f"| {total['pub_items']} | {total['tests']} |"
    )


if __name__ == "__main__":
    main()
