# Contributing

`docs/design.md` explains how teleprompt works and why. Read the section
covering what you're changing before you change it.

## Build and test

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The workspace builds and tests offline, with no network, browser or Node.
Tests that need an external tool skip themselves when it's missing. CI sets
these variables so a skip there is a failure:

| variable | turns a skip into a failure when |
|---|---|
| `TELEPROMPT_REQUIRE_FFMPEG` | ffmpeg is missing |
| `TELEPROMPT_REQUIRE_VHS` | vhs is missing |
| `TELEPROMPT_REQUIRE_CAPTURE` | the manual renders any shot as a slate |

`crates/teleprompt-cli/tests/golden.rs` pins `plan` output for every example
project, and every command's failure output and exit code. When a change is
meant to alter output, review the new snapshots with `cargo insta review`
(or rerun with `INSTA_UPDATE=always`) and commit them with the change. When
a change isn't meant to alter output, the snapshots must not move.

The oldest supported Rust is 1.85, the `rust-version` in `Cargo.toml`, and
CI checks it with `cargo +1.85 check --workspace --locked`.

## Changes

- **Keep one theme per commit.** A refactor keeps behaviour identical, and
  a fix changes it on purpose and says so. Don't mix the two.
- **Every fix comes with a test that fails without it.**
- **Keep functions small**: at most 100 lines and seven arguments. Clippy
  enforces both (`too_many_lines`, `too_many_arguments`), so split along
  the steps a function performs rather than raising the limit.
- **Keep pull requests small**: roughly 500 changed lines, deletions aside.
- **Measure structural changes.** `python3 tools/metrics.py` prints lines,
  comment ratio, longest function, `pub` items and test count per crate. A
  cleanup quotes the numbers before and after.

## Comments

A comment explains what the code can't say itself: why it's this way, what
would go wrong otherwise, and what a reader might wrongly assume.

- **Doc comments describe what is true now.** How the code got this way
  belongs in the commit message. Don't write "used to", "an earlier draft"
  or "was changed to".
- **Keep each piece of rationale in one place.** If the reason is in
  `docs/design.md`, link to it (`docs/design.md#quiet-window`) rather than
  restating it. If it's in another doc comment, link to that item.
- **Don't cite process.** No milestone names, task numbers, review rounds
  or finding ids. Nobody reading the code can look them up.
- **Don't restate the code.** A comment that repeats the function name, the
  types or the next line adds reading without adding information.
- **Keep them short.** Aim for no comment block over 12 lines, and comments
  no more than a quarter of a crate's lines. Anything longer usually belongs
  in `docs/design.md`.
- **Keep them true.** A change that makes a comment false fixes the comment
  in the same commit.

The same applies to test names and assertion messages. A test's doc comment
says what behaviour it pins and why that matters, not how the bug was found.
