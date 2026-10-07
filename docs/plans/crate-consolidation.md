# Crate consolidation: 26 crates to 17

Status: planned. Each step is one commit, done in order, with the checks
in "Every step" run before committing. A step that is green can be pushed
on its own; nothing here depends on a later step.

## Why

The workspace has 26 crates and 70 dependency edges. Measured against
ripgrep and uv, the layering is right but the granularity is not: seven
crates hold one built-in scene plugin each with no distinct dependencies,
the scene contract and the scene SDK are two crates with one consumer
between them, the compiler is three crates (schedule, compile, manifest)
that only ever ship together, and the registry is a 188-line crate whose
only reader is the command line. Each is a boundary Cargo enforces that
nothing needs enforced.

## Target

| crate | one-line reason to exist |
|---|---|
| `teleprompt-core` | ids, diagnostics, config, time, progress, and `tool`: what every layer speaks |
| `teleprompt-script` | the language: parse, resolve, program, lint, edit |
| `teleprompt-manifest` | the published output format: the contract between the compiler and any renderer |
| `teleprompt-pipeline` | script to timeline and manifest: today's `schedule` + `compile` |
| `teleprompt-voice` | the voice contract, cache, takes |
| `teleprompt-scene` | the scene plugin SDK: contract, capture, protocol, recording |
| `teleprompt-render` | manifest to video; depends on `manifest`, never on `pipeline` |
| `teleprompt-listen` | following a reader by ear |
| `teleprompt-scenes` | the seven built-in scene plugins, one module each |
| `teleprompt-voices` | the built-in voices (unchanged) |
| `teleprompt-project` | a project and its scripts: dub, capture, build, edit, translate |
| `teleprompt-serve` | the prompter (axum) |
| `teleprompt-draft` | a script from what you have: document, transcript, recording, session |
| `teleprompt-lsp` | the language server (lsp-server, lsp-types) |
| `teleprompt-setup` | the machine: tools and models, and installing them (tabled) |
| `teleprompt-cli` | the binary, composing the rest; holds the registry |
| `teleprompt-testkit` | test fixtures |

Dependency rules after the change (`tools/check_deps.py` is updated to
exactly this; anything else fails CI):

```
script, manifest, voice, scene, listen        -> core
pipeline                                       -> core, script, manifest, scene, voice
render                                         -> core, manifest
scenes                                         -> scene
voices                                         -> core, voice
setup                                          -> core
project                                        -> core, script, manifest, pipeline, scene, voice, render, setup
lsp                                            -> core, script, project
draft                                          -> core, script, listen, voice, scene, project, setup
serve                                          -> core, script, listen, voice, project, setup
cli                                            -> everything above (it is the composition root)
testkit                                        -> core
```

`render -> manifest` and never `pipeline` is the one wall kept on
purpose: an outside renderer reads the manifest and must not need the
scheduler. `pipeline -> scene` is for the scene contract (`SceneCompiler`),
as `compile -> scene` is today; it must not reach `scene::protocol`
(enforced by review, not Cargo).

## Two decisions already taken

1. **`draft` stays.** Its tool knowledge is through the `Recording` trait,
   not hardcoded, except `record`'s microphone and PTY handling, which is
   scene-SDK work and moves there (step 6). The Linux app's session mode
   is built on `teleprompt record`, so the command is kept.
2. **`plugin` is renamed `scene`** and absorbs the contract crate of that
   name. Voices are pluggable too, through `voice`; this crate is only
   about scenes. `scene`/`scenes` mirrors `voice`/`voices`.

## Every step

Run before each commit, in this order; all must be clean:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p teleprompt-draft -p teleprompt-serve -p teleprompt-setup -p teleprompt-cli --features listen --all-targets -- -D warnings
python3 tools/check_deps.py
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --lib
cargo test --workspace
```

Also, for every crate renamed or removed, grep the whole repository
(not `target/`, `node_modules/`) for the old crate name in both spellings
(`teleprompt-x`, `teleprompt_x`) and fix each hit: Cargo manifests,
`tools/check_deps.py`, `.github/workflows/*.yml` path filters and
`-p` flags, `docs/design.md` (the crate table and prose), `docs/guide/*.md`,
`CONTRIBUTING.md`, `examples/*/`, and doc comments. The disk in the
cloud environment fills during full test runs; delete large test
binaries under `target/debug/deps` between runs if `No space left`
appears.

Commit messages: say what moved and why in plain prose, no bullet lists,
no model names. Push to `claude/remotion-teleprompt-plugin-uydkz3` and
to `main`.

## Steps

### 1. `tool` moves to `core::tool`

- `git mv crates/teleprompt-plugin/src/tool.rs crates/teleprompt-core/src/tool.rs`;
  add `pub mod tool;` to `core/src/lib.rs`.
- Replace `teleprompt_plugin::tool::` with `teleprompt_core::tool::` in
  every crate that depends on `core`. The seven scene plugin crates do
  not (they depend on the SDK alone), so they write
  `teleprompt_plugin::core::tool::`, the SDK's re-export of core.
- `tool.rs` had two methods tied to the plugin wire protocol, `to_wire`
  and `from_wire`. They stay in the plugin crate as `Need::of(&Tool)` and
  `Need::into_tool(&self)` in `protocol/mod.rs`.
- `setup` and `voices` lose nothing; `setup` already depends on `core`.

### 2. `scene` absorbs `plugin`; rename

- `git mv crates/teleprompt-plugin crates/teleprompt-scene-sdk` (temporary
  name), then move `crates/teleprompt-scene/src/{contract,mock}.rs` into
  it as `src/contract.rs` and `src/mock.rs`, delete the old
  `teleprompt-scene`, and `git mv crates/teleprompt-scene-sdk crates/teleprompt-scene`.
- `Cargo.toml`: `name = "teleprompt-scene"`. Drop the `teleprompt-scene`
  dependency line. Keep `[features]` as they are.
- `lib.rs`: replace `pub use teleprompt_scene as scene;` with
  `mod contract; mod mock; pub use contract::*; pub use mock::MockScene;`
  (keep whatever the old `teleprompt-scene/src/lib.rs` exported, at the
  same paths minus the `scene::` prefix). Keep `pub use teleprompt_core as core;`.
- Everywhere: `teleprompt_plugin` -> `teleprompt_scene`;
  `teleprompt_scene::scene::X` and `teleprompt_plugin::scene::X` -> `teleprompt_scene::X`.
  `teleprompt-compile` depended on `teleprompt-scene` for the contract: its
  manifest line now points at the merged crate, same name.
- `docs/guide/scene-plugins.md` and `docs/design.md` describe
  `teleprompt-plugin` and `teleprompt-scene` as two crates; rewrite to one.
- `.github/workflows/ci.yml` names `teleprompt-scene` twice: check the
  context still holds.

### 3. `pipeline` = `schedule` + `compile`

- New crate `crates/teleprompt-pipeline` with `src/schedule/` (the former
  `schedule/src/*`, its `lib.rs` becoming `schedule/mod.rs`) and
  `src/compile/` (likewise). `lib.rs`:
  `pub mod compile; pub mod schedule;` plus `pub use` of what each old
  `lib.rs` exported, at the paths callers use today:
  `teleprompt_schedule::X` -> `teleprompt_pipeline::schedule::X`,
  `teleprompt_compile::X` -> `teleprompt_pipeline::compile::X`.
  Inside the moved files, `crate::` -> `crate::schedule::` or
  `crate::compile::`, and `teleprompt_schedule::` inside compile ->
  `crate::schedule::`.
- Dependencies: the union of the two old manifests. `manifest` stays a
  separate crate and a dependency.
- Tests: `schedule/tests/*` and `compile/tests/*` move to
  `pipeline/tests/`, prefixed `schedule_` and `compile_` where names
  collide.
- `project` re-exports (`lib.rs`) point at the new paths; `cli`'s dev-dep
  on `compile` becomes `pipeline`.
- `.github/workflows/linux-app.yml` lists `crates/teleprompt-schedule/**`
  in path filters: replace with `crates/teleprompt-pipeline/**`.

### 4. `scenes` = the seven built-in scene plugins

- New crate `crates/teleprompt-scenes`, `src/lib.rs` with
  `pub mod asciinema; pub mod desktop; pub mod media; pub mod playwright; pub mod remotion; pub mod slidev; pub mod vhs;`.
  Each old crate's `src/` becomes `src/<name>/` (its `lib.rs` as
  `mod.rs`; `crate::` inside -> `crate::<name>::`). Each old crate's
  `tests/` moves to `scenes/tests/` prefixed `<name>_`.
- Dependencies: `teleprompt-scene`, `serde_json` (asciinema, remotion),
  dev: `teleprompt-testkit`. Nothing else; verified in the survey.
- `cli` has dev-deps on `teleprompt-asciinema`; point at `scenes`.
- `tools/check_deps.py`: drop `PLUGIN_CRATES`; `"scenes": {"scene"}`.
- `docs/guide/scene-plugins.md` "Compiled in" section: a built-in plugin
  is now a module of `teleprompt-scenes` plus one line in the registry.
  `docs/design.md` crate table: one row for `scenes`.
- `.github/workflows/*.yml`: any `crates/teleprompt-<plugin>/**` path
  filter becomes `crates/teleprompt-scenes/**`.
- Do not merge `examples/*`: those are out-of-process example plugins and
  stay as they are.

### 5. The registry moves into `cli`

- `git mv crates/teleprompt-registry/src crates/teleprompt-cli/src/registry`
  (`lib.rs` -> `mod.rs`); `pub mod registry;` in `cli/src/lib.rs`.
  `teleprompt_registry::registry()` -> `teleprompt_cli::registry::registry()`
  in every integration test (23 files) and in `cli/src/cli.rs`.
- `cli/Cargo.toml` gains `teleprompt-scenes` and `teleprompt-voices` as
  normal dependencies; `teleprompt-registry` goes.
- `docs/guide/voices.md` and `docs/guide/scene-plugins.md` tell a
  contributor where to register a voice or plugin: now
  `crates/teleprompt-cli/src/registry/{voices,scenes}.rs`.
- `project::registry::Registry` (the type) stays where it is; only the
  construction moves.

### 6. Recording a session is the scene SDK's

- Move from `draft/src/record.rs` into `scene/src/record.rs` (which
  already holds the `Recorder` trait, `Start`, `Recorded`) the parts that
  run a tool and a microphone: starting the recorder in a PTY or a
  window, ffmpeg microphone capture, the stop key and SIGTERM handling,
  and the status file. Expose one function, roughly
  `pub fn record(recorder: &Recording, into: &Path, reporter: &dyn Reporter) -> Result<Recorded, String>`,
  returning the recording and the voice file. Keep the behaviour and the
  status-file JSON byte-identical: `apps/linux/src/tools.rs` and the
  session mode read it.
- `draft::record::run_record` keeps choosing the recorder from the
  registry, calls `teleprompt_scene::record::record`, then drafts as now.
  After this, `grep -n 'Command::new' crates/teleprompt-draft/src` must
  show only the ffmpeg decode in `document.rs`.
- `signal-hook` moves from `draft`'s manifest to `scene`'s.
- Tests `cli/tests/record.rs` and `record_listen.rs` must pass unchanged.

### 7. Docs, diagram, checker, CI

- `docs/design.md`: rewrite the crate table to the Target above, the
  dependency paragraph to the rules above, and the "Scene plugins and
  voices are named only in `registry`" paragraph to name
  `teleprompt-cli/src/registry`.
- `tools/check_deps.py`: the rules above, verbatim; run it.
- `CONTRIBUTING.md`: if it names crates, update.
- `.github/workflows`: every `-p teleprompt-…` flag and path filter
  checked against `ls crates`.
- `cargo metadata --no-deps | jq '.packages | length'` reports 17.

## Done when

- `ls crates` shows exactly the 17 crates in Target.
- `tools/check_deps.py` passes with the rules above and nothing more.
- All checks in "Every step" are clean on the default and `listen` builds.
- CI is green on `main` for the final commit (app, test, licenses,
  speech, msrv).
- `docs/design.md`'s crate table matches `ls crates`.
