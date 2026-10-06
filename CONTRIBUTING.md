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
| `TELEPROMPT_REQUIRE_LISTEN` | `TELEPROMPT_LISTEN_MODEL` is not set |

The speech recognizer is opt-in, because sherpa-onnx downloads its native
library when it builds. Its tests run a real model over a recorded reading,
following it and transcribing it into timed words (the `Speech` workflow
runs them in CI):

```bash
TELEPROMPT_LISTEN_MODEL=path/to/sherpa-onnx-streaming-zipformer-en-2023-06-26 \
  cargo test -p teleprompt-listen --features sherpa
```

The model is sherpa-onnx's streaming English zipformer; the smaller 20M
model misses the first words of a stream, and the test catches it. Behind a
proxy its build script may not trust, download the native archive yourself
and point `SHERPA_ONNX_ARCHIVE_DIR` at the directory holding it.

`crates/teleprompt-cli/tests/golden.rs` pins `check` and `plan` output for
every example project whose plugins are built in (all but
`plugin-authors`), and every command's failure output and exit code. When
a change is meant to alter output, review the new snapshots with `cargo insta review`
(or rerun with `INSTA_UPDATE=always`) and commit them with the change. When
a change isn't meant to alter output, the snapshots must not move.

A test that needs a directory takes one from
`teleprompt_testkit::test_dir("name")` (a dev-dependency). It is deleted
when the value drops, even if the test panics, so keep it bound for as long
as anything uses the path: return it from a helper alongside what was built
in it, and bind it as `_dir`, not `_`, which drops it at once.

The oldest supported Rust is 1.88, the `rust-version` in `Cargo.toml`, and
CI checks it with `cargo +1.88 check --workspace --locked`.

## Changes

- **Keep one theme per commit.** A refactor keeps behaviour identical, and
  a fix changes it on purpose and says so. Don't mix the two.
- **Work test-first.** Write the test, run it and watch it fail for the
  reason you expect, then write the code that makes it pass. That goes for
  features as much as fixes: a test written after the code has never been
  seen to fail, so it proves less.
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
