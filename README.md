# teleprompt

Compile narrated videos from version-controlled Markdown.

A script's prose is its narration; fenced `teleprompt` blocks are its visuals.
Narration duration drives visual pacing, so editing a paragraph changes the
rhythm of the video — and `teleprompt diff` tells you exactly how before you
render anything.

**Status: M0.** The compiler and the feedback loop work end to end: parsing,
narration timing, and timeline scheduling are all real. There is no video
output yet — no rendering, no scene adapters beyond the mock one used for
testing, no ffmpeg. See
`docs/superpowers/specs/2026-08-15-teleprompt-design.md` for the full design
and milestone plan.

## Try it

```bash
cargo run -- new demo
cargo run -- plan demo/scripts/demo.md
cargo run -- diff demo/scripts/demo.md
```

`new` scaffolds a project with a demo script. `plan` compiles it into a
timeline and prints one line per beat, with its narration duration and
policy. `diff` compares that timeline against whatever is committed — on a
fresh project there's nothing committed yet, so every beat shows up as
`added`.

Now edit a paragraph in `demo/scripts/demo.md`, run `diff` again, and watch
the transitions move — that's the whole feedback loop. `plan`'s JSON output
is what actually gets committed to `demo/timelines/`, so committing it after
`plan` is what makes the next `diff` compare against something real:

```bash
cargo run -- plan demo/scripts/demo.md --format json > demo/timelines/demo.en.json
# edit demo/scripts/demo.md
cargo run -- diff demo/scripts/demo.md
```

## Commands

| command | does |
|---|---|
| `new <path>` | scaffold a project |
| `check <script>` | parse and validate; no side effects, no cost |
| `plan <script>` | compile the timeline and print it |
| `diff <script>` | compare against the committed timeline |
| `doctor` | report the environment teleprompt can see |

All commands accept `--format json`.

## Building and testing

No network, no browser, no Node, and no ffmpeg are required — the whole
workspace builds and tests offline:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

`tests/fixtures/tour.md` is a realistic multi-chapter script exercised by
`crates/teleprompt-cli/tests/end_to_end.rs`, the acceptance suite that
answers M0's defining question: does editing prose produce a legible, useful
diff of the video's pacing?
