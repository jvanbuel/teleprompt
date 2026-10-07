# Round five: one catalogue, in two halves

Status: planned. Follows `seams.md`, which is done. Crate count stays 17.
One commit per step, in order; the checks in "Every step" clean before
each; nothing depends on a later step.

## Why

The review after round four measured every boundary. The layers are
natural. The forced boundaries all trace to one type: `Registry`, in the
project crate, whose fields come from `scene` and `voice` and whose
methods are half scene lookups and half voice lookups. Because it lives
above both, four workarounds exist:

1. `setup` cannot see it, so the command line copies it into `Shipped`
   and `Needs`, field by field.
2. The project's voice report is copied into setup's `ProjectVoice` the
   same way.
3. `project::voice::Backends`, 300 lines of voice runtime (resolve an id,
   probe a server, concurrency, cloning), lives in `project` because its
   one tie to the project crate is `Registry`.
4. `own_server` is a bare function pointer so the registry can reach the
   openai module without naming it.

Two leftovers from round four sit beside these: `SceneCompilers` has one
implementor and both sides of the seam it bridged are now one crate; and
16 of `pipeline`'s 35 public items are referenced by nothing outside it,
made public when schedule and compile were separate crates. And one rule
the compiler cannot hold, that the pipeline uses only the scene contract,
is enforced by review alone.

## Target

The catalogue of what a build has lives where its halves are defined:
scene plugins in `scene`, voices in `voice`. `Registry` is the pair, held
by the project crate for the code that needs both. `setup` takes the two
halves from the crates it may already depend on. Nothing is copied.

Dependency rules after the change. Only `setup` changes: it gains `voice`.

```
script, manifest, voice, scene, listen   -> core
pipeline                                 -> core, script, manifest, scene, voice
render                                   -> core, manifest
scenes                                   -> scene
voices                                   -> core, voice
setup                                    -> core, scene, voice
project                                  -> core, script, manifest, pipeline, scene, voice, render
lsp                                      -> core, script, project
draft                                    -> core, script, listen, voice, scene, project, setup
serve                                    -> core, script, listen, voice, project, setup
cli                                      -> everything above
testkit                                  -> core
```

## Every step

As in the earlier plans: `cargo fmt --all`, both clippy builds (default,
and `--features listen` on draft, serve, setup, cli),
`python3 tools/check_deps.py`, strict rustdoc, `cargo test --workspace`.
Grep for every moved name in both spellings. Delete large test binaries
under `target/debug/deps` between runs if the disk fills. Commit messages
in plain prose, no bullet lists, no model names. Push to
`claude/remotion-teleprompt-plugin-uydkz3` and to `main`.

## Steps

### 1. The voice half: `teleprompt_voice::catalogue`

- New module `voice/src/catalogue.rs`:

  ```rust
  /// A voice this build ships, and what it needs that teleprompt does
  /// not: a server the author runs, or a key.
  pub type Shipped = (Provider, &'static Tool);

  /// How a `backends:` key naming no shipped voice is built: a server of
  /// the author's under the name they gave it, from its settings. The
  /// same shape as `Provider::build`, with the name it was given.
  pub type Fallback = fn(&str, &serde_yaml::Value) -> Result<Arc<dyn VoiceBackend>, String>;

  /// Every voice this build has, `null` aside.
  #[derive(Clone, Copy)]
  pub struct VoiceCatalogue {
      pub shipped: &'static [Shipped],
      pub fallback: Fallback,
  }
  ```

  with `ids()`, `tools()` (the `&Tool` of each), `is_shipped(name)`, moved
  from `project::registry` (`voice_tools`, `shipped_voices`,
  `is_shipped_voice`).
- Move `project/src/voice/mod.rs` to `voice/src/backends.rs`: `Backends`,
  `backends_for` (now `Backends::new(catalogue: &VoiceCatalogue, settings, config_file)`),
  `VoiceStatus`, and `status` as `Backends::status(&self, backend: &str) -> VoiceStatus`
  (what `project::voice::status` did after reading the backend name from
  the project's config; the project crate keeps a one-line
  `Project::voice_status()` that reads the name and calls it). `Voices`
  and whatever else that module holds move with it. `NullVoice`,
  `VoiceRegistry`, `Diagnostic` are already reachable from `voice`.
- `project/src/voice/` keeps only what is project-specific. If nothing
  is, delete the module and re-export `teleprompt_voice::backends::Backends`
  where `project` used `crate::voice::Backends`.
- `voice/Cargo.toml` gains `serde_yaml` if it lacks it (the settings are
  YAML values); it already depends on `core` for `Tool`.
- Tests: `cli/tests/backend_selection.rs`, `voice_selection.rs`,
  `setup_voice.rs` reach `Backends` and `VoiceStatus` through `voice`.

### 2. The scene half, and the trait that outlived its seam

- `ScenePlugins` (in `scene/src/scene_plugin.rs`) gains a
  `shipped: Vec<&'static str>` field, set by whoever builds it (the cli's
  registry module today), and the methods moved from `project::registry`:
  `is_shipped(name)`, `needs_of(name) -> Option<Vec<&str>>`, `tools()`,
  `recorders() -> Vec<Recording>`. `Recording` (a plugin name with its
  `&'static dyn Recorder`) moves to `scene::record`.
- Delete `SceneCompilers`. `pipeline::compile` takes `&ScenePlugins` where
  it took `&dyn SceneCompilers`; `ScenePlugins::compiler(name)` and
  `names()` keep their signatures as inherent methods. There was one
  implementor, so nothing else changes.
- `Registry::mock()` builds a `ScenePlugins` with the mock plugin and
  `shipped: vec!["mock"]`, as now.

### 3. `Registry` is the pair; `setup` takes the halves

- `project::registry::Registry` becomes

  ```rust
  #[derive(Clone, Copy)]
  pub struct Registry {
      pub scenes: &'static ScenePlugins,
      pub voices: &'static VoiceCatalogue,
  }
  ```

  with `backends(settings, file) -> Backends` delegating to
  `Backends::new(self.voices, …)`, `default_backends()`, and `mock()`.
  Every other method is gone; callers use `registry.scenes.…` or
  `registry.voices.…`. The `Voice` and `OwnServer` type aliases are gone.
- The cli's `registry/mod.rs` builds `VoiceCatalogue { shipped: VOICES, fallback: teleprompt_voices::openai::endpoint }`
  and `ScenePlugins` with its `shipped` list; `registry/voices.rs` returns
  `Vec<teleprompt_voice::catalogue::Shipped>`.
- `setup`: delete `Shipped` and `Needs`. `Setup` holds
  `scenes: &'static ScenePlugins` and `voices: &'static VoiceCatalogue`;
  `Setup::detect(scenes, voices, project_root)`. The free functions
  `resolve`, `tools`, `download_mb` become methods on `Setup` (they took
  `&Shipped`; they read `self.scenes` and `self.voices` now). `plugins.rs`
  reads `setup.scenes` for the built-in rows. `SetupReport.voice` is
  `Option<teleprompt_voice::backends::VoiceStatus>`; `ProjectVoice` is
  deleted and the cli's field-by-field copy with it.
- The cli's `registry::shipped()` and `setup_here()` collapse to
  `setup_here(registry) = Setup::detect(registry.scenes, registry.voices, root_here(registry))`.
  `ask.rs` and `commands/setup.rs` call `setup.resolve(…)` and
  `setup.download_mb(…)`.
- `serve`: `Server` holds the `Registry` it already has; `setup()` calls
  `Setup::detect(self.registry.scenes, self.registry.voices, root_here(self.registry))`.
  The `shipped` field and the `run_serve` parameter added in round four
  go away again.
- `tools/check_deps.py`: `"setup": {"core", "scene", "voice"}`.
- Tests under `cli/tests/setup*.rs` that built a `Shipped` by hand build a
  `Setup` from the registry's halves instead.

### 4. Pipeline's surface, and the rule the checker can hold

- In `pipeline`, make `pub(crate)` or private each of: `ActionEntry`,
  `ActionInput`, `CAPTURE_RECIPE`, `ChangedBeat`, `ChangedTransition`,
  `Layout`, `NarrationEntry`, `NarrationInput`, `Pacing`,
  `ReorderedBeat`, `fit_line`, `layout_at`, `over_length`,
  `padded_duration_ms`, `version_of`, `word_offset_ms`, unless a test
  under `pipeline/tests` uses it, in which case it stays `pub` and gets a
  `#[doc(hidden)]`. Re-run the measurement: items public and unreferenced
  outside the crate should be the test-only ones.
- `tools/check_deps.py` gains one rule beside the edge table: the files
  under `crates/teleprompt-pipeline/src` must not contain
  `scene::protocol`, `scene::capture` or `scene::record` (the compiler
  plans; it never runs a tool). Implement as a grep over those files,
  failing with the file and line. `cargo-deny`'s `wrappers` rule was
  considered for the edge table and not adopted: it cannot express this
  module-level rule, and one checker is better than two.
- `docs/design.md`: the sentence "Cargo cannot enforce that one, so
  review does" becomes "`tools/check_deps.py` holds it".

### 5. Docs and the page

- `docs/design.md` crate table: `voice` gains "the catalogue of voices a
  build ships and the backends a project's settings make of them";
  `scene` gains "and `ScenePlugins`, the catalogue of a build's scene
  plugins"; `project`'s row drops the registry description to "holds the
  two catalogues as one `Registry`"; `setup`'s row says it takes the two
  catalogues.
- The dependency paragraph: `setup` is on `core`, `scene` and `voice`.
- The architecture page: round six list with these five moves; `setup`
  gains an edge to `voice`; `project` loses the voice runtime from its
  label (and roughly 300 lines), `voice` gains it.

## Done when

- `grep -rn "Shipped\|Needs\|ProjectVoice\|OwnServer\|SceneCompilers" crates/*/src`
  finds only `teleprompt_voice::catalogue::Shipped`.
- `project/src/registry.rs` defines a two-field `Registry` and nothing
  that reads inside either half.
- `tools/check_deps.py` passes with the Target rules and the pipeline
  grep, and fails if a line `use teleprompt_scene::protocol` is added to
  `pipeline/src/compile/mod.rs` (try it, then revert).
- All checks in "Every step" clean; CI green on `main`.
