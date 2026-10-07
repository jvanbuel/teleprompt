# Round four: the seams

Status: in progress. Follows `crate-consolidation.md`, which is done. The crate
count stays at 17; every step here is about an edge or a type at a
boundary. One commit per step, in order, with the checks in "Every step"
clean before each. Nothing depends on a later step.

## Why

With the layering settled, three flaws remain, all at seams:

1. **The domain depends on the installer.** `project -> setup` exists for
   one report type (`ProjectVoice`) and two constructors (`Registry::shipped`,
   `Registry::setup_here`). A project should not know how tools are installed.
2. **"What this build has" is built in two shapes.** `Registry` (project)
   and `Shipped` (setup) describe the same list, and `Shipped` is derived
   from `Registry` inside the project crate. The derivation belongs in the
   one place that composes, the command line.
3. **Typed errors stopped at the pipeline.** Eleven public functions in
   `setup`, `draft`, `project` and `lsp` still return `Result<_, String>`.
   `draft` is the crate most likely to gain a second front end.

And one that is not a flaw yet but costs every crate a rebuild: `core`
is 2.6k lines, 750 of them the project-file schema, which only crates
that already depend on `script` read. The scene, manifest and render crates
link it for a handful of value types.

## Target

Dependency rules after the change. Only `project` changes; it loses `setup`.

```
script, manifest, voice, scene, listen   -> core
pipeline                                 -> core, script, manifest, scene, voice
render                                   -> core, manifest
scenes                                   -> scene
voices                                   -> core, voice
setup                                    -> core, scene
project                                  -> core, script, manifest, pipeline, scene, voice, render
lsp                                      -> core, script, project
draft                                    -> core, script, listen, voice, scene, project, setup
serve                                    -> core, script, listen, voice, project, setup
cli                                      -> everything above
testkit                                  -> core
```

## Every step

As in `crate-consolidation.md`: `cargo fmt --all`, both clippy builds
(default, and `--features listen` on draft, serve, setup, cli), 
`python3 tools/check_deps.py`, strict rustdoc, `cargo test --workspace`.
Grep the repository for every moved name in both spellings. Delete large
test binaries under `target/debug/deps` between runs if the disk fills.
Commit messages in plain prose, no bullet lists, no model names. Push to
`claude/remotion-teleprompt-plugin-uydkz3` and to `main`.

## Steps

### 1. `project` no longer depends on `setup`

What `project` takes from `setup` today, and where each goes:

- `ProjectVoice` (the type `voice::status` returns): becomes
  `teleprompt_project::voice::VoiceStatus`, same three fields and docs.
  `setup::SetupReport.voice` keeps the field, typed as the report's own
  `Option<serde_json::Value>`? No: keep it typed. `SetupReport` is built by
  the command line, which has both crates; give `SetupReport` a generic or
  move the field's type to `setup::VoiceStatus { backend, answer, problems }`
  with a `From<teleprompt_project::voice::VoiceStatus>` impl in the cli.
  Pick the second: `setup` keeps a plain struct of its own, and the cli
  converts. Both structs serialize identically, so `setup`'s JSON output and
  the test `tests/setup_voice.rs` do not change.
- `Registry::shipped()` and `Registry::setup_here()`: move to the cli's
  `registry` module as free functions `shipped(registry) -> Shipped` and
  `setup_here(registry) -> Setup`. `Needs` and `Shipped` stay `setup`'s.
- `serve` calls both through the registry (`routes.rs`: `uses_route`,
  `install_route`). It cannot reach the cli, so `Server` takes a
  `teleprompt_setup::Setup` at construction, built by the cli beside the
  `Registry`, and the two routes use `server.setup` (`uses()`, `shipped`,
  `install`). `run_serve` gains the parameter; the cli's `serve` command
  and every test that builds a `Server` pass `setup_here(registry)`.
- `tests/setup*.rs` and `ask.rs` in the cli switch from
  `registry().shipped()` to `registry::shipped(registry())`.
- `tools/check_deps.py`: `project` loses `"setup"`.
- As done: a `From` impl in the cli would break the orphan rule, so the
  cli converts `VoiceStatus` to `setup::ProjectVoice` field by field. The
  prompter holds the `Shipped` list rather than a `Setup`, and detects the
  machine per request, as it did, so a tool installed from the page is
  found by the next request. Both use `project::root_here`.
- `docs/design.md`: the dependency paragraph says `setup` is a leaf on
  `core` and `scene`; the `teleprompt-setup` row drops "the project crate
  depends on it".

### 2. The project-file schema is `script::config`

`core::config` splits. What stays in `core::config` is the vocabulary other
crates speak without reading a file:

- `SceneConfig` (scene, pipeline, project), `Transition`, `TransitionKind`,
  `OtherKind`, `TransitionDuration` (manifest, render, pipeline),
  `TimingConfig`, `OutputConfig`, `TransitionConfig`, `Resolution` (pipeline,
  project), and their impls.

What moves to a new `teleprompt_script::config` module is the file:

- `Config` and its `Default`, `TranslateConfig`, `TRANSLATE_TIMEOUT_MS`,
  `Locales`, `VoiceConfig`, every `Partial*` type, `ConfigError`,
  `PartialConfig::merged` and the rest of its impl, `VoiceConfig`'s impl,
  `locale_problem`.
- The moved code imports the vocabulary from `teleprompt_core::config`.
- `script` already depends on `core`; `resolve` already reads `Config`.
- Callers: `pipeline`, `project`, `lsp` and `cli` replace
  `teleprompt_core::config::{Config, PartialConfig, …}` with
  `teleprompt_script::config::…`. `manifest`, `render` and `scene` change
  nothing. `toml` and `serde_yaml` move from `core`'s manifest to
  `script`'s if nothing else in `core` uses them (check `core/src/*.rs`
  first; `attrs.rs` and `said.rs` may).
- Tests under `core/tests` that exercise the file schema move to
  `script/tests/config.rs`.
- Done when `wc -l crates/teleprompt-core/src/config.rs` is under 300 and
  `cargo tree -p teleprompt-scene -e normal` shows no `toml`.

### 3. Typed errors at the last four boundaries

Each crate gets one `thiserror` enum, and the command line maps it in one
place. The eleven functions:

**setup** (`src/lib.rs`): `speech_model` returns `Result<PathBuf, SetupError>`
with `SetupError::NoSpeechModel { models_dir: PathBuf }` whose `Display` is
today's message verbatim. `install`, `run_aloud`, `run_quietly`,
`without_a_terminal` already return `String` internally; they may stay
private-shaped, but `install` is public: give it
`SetupError::Install { tool: String, why: String }`.

**draft** (`src/import.rs`, `src/record.rs`, `src/listening.rs`): one
`DraftError` enum in `src/lib.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    #[error("{0}")] WouldReplace(String),            // refuse_to_replace
    #[error("this teleprompt was built without a speech recognizer: rebuild it with `--features listen`")]
    NoRecognizer,
    #[error("no {what} model at {}", dir.display())] NoModel { what: &'static str, dir: PathBuf },
    #[error("cannot record with {plugin}: {why}")] Recorder { plugin: &'static str, why: String },
    #[error("the drafted {} does not compile, which is a bug in `import`:\n{}", path.display(), problems.render().join("\n"))]
    DraftDoesNotCompile { path: PathBuf, problems: Diagnostics },
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error("{0}")] Other(String),                   // everything not yet classified
}
```

`run_import`, `run_record`, `draft_session`, `refuse_to_replace` and
`hear` return it. Start with every `Err(format!(…))` wrapped in `Other`,
then move the cases named above into their variants; the message text must
not change, since `tests/import*.rs` and `tests/record*.rs` match on it.
`listening::hear` under `#[cfg(not(feature = "listen"))]` returns
`NoRecognizer`. The status file in `run_record` writes `e.to_string()`, as
now.

**project**: `dub::publish::publish` returns `Result<Published, Failure>`
(it is called from `dub`, which already returns `Failure`, so the map at
the call site disappears). `build::parse_resolution` returns
`Result<(u32, u32), ResolutionError>` with one variant whose `Display` is
the current message; the cli's `FrameArgs` is its only caller. The
translate providers `OpenAi::new` and `Claude::from_env` return
`TranslateError` (`MissingSetting { key }`, `MissingEnv { var }`), which
`Translator::new` maps into `Failure::Runtime` as it does the string now.

**lsp**: `run_lsp` returns `Result<(), LspError>` with
`#[error(transparent)] Protocol(#[from] lsp_server::ProtocolError)` and
`Io(#[from] std::io::Error)`, whichever the inner `run` yields.

**cli**: `output.rs` gets `From<DraftError>`, `From<SetupError>`,
`From<LspError>` for `Outcome`, each `RuntimeFailure(e.to_string())`, and
the `map_err(runtime_failure)` calls on those paths go. Exit codes and
printed text are unchanged; the integration tests prove it.

Done when `grep -rE "pub (async )?fn [^{]*Result<[^>]*, String>" crates/*/src`
matches only functions inside `scenes`, `voices` and `scene::protocol`,
where no caller distinguishes cases.

### 4. Docs and the page

- `docs/design.md`: the crate table rows for `core` (drops "the config"),
  `script` (gains "the project file, `teleprompt.toml`, and its layers"),
  `setup` and `project` as step 1 says; the dependency paragraph.
- `tools/check_deps.py` already updated in step 1; confirm it passes with
  nothing else changed.
- The architecture page (the artifact) notes the dropped edge and the
  config move in a round five list.

## Done when

- `tools/check_deps.py` passes with the rules in Target, and `project`'s
  `Cargo.toml` has no `teleprompt-setup` line.
- `core/src/config.rs` is under 300 lines; `teleprompt-scene` does not link
  `toml`.
- The only `Result<_, String>` on public functions are in `scenes`,
  `voices` and `scene::protocol`.
- All checks in "Every step" are clean; CI green on `main`.
