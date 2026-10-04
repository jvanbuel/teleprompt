//! What names an item's picture.
//!
//! A scene is a session: the items of a walkthrough continue one another,
//! and the screen at item *N* is the accumulation of items 1..*N*. So a
//! clip is not identified by its own tape — it is identified by its tape
//! and every tape before it in that session. This is OCI's chain ID for
//! the same reason OCI needs one.
//!
//! The tests are about identity, not pixels: nothing here captures
//! anything. They are the arithmetic a capture stage will trust.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use teleprompt_cache::VoiceCache;
use teleprompt_compile::{compile, CompileOutput, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::resolve;
use teleprompt_core::{BlockId, Hash};
use teleprompt_plugin::scene::SceneRegistry;
use teleprompt_voice::WpmEstimator;

fn run(src: &str) -> CompileOutput {
    run_with(src, &SceneRegistry::with_builtins())
}

fn run_with(src: &str, registry: &SceneRegistry) -> CompileOutput {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let cache =
        VoiceCache::new(std::env::temp_dir().join(format!("tp-chain-{}-{n}", std::process::id())));
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
        takes: &teleprompt_voice::takes::Takes::default(),
    };
    let parsed = parse_script(src).expect("fixture parses");
    let program = resolve(
        &parsed,
        "tour.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect("fixture resolves");
    compile(&program, registry, &ctx, Path::new("."), "0.1.0").expect("fixture compiles")
}

/// Every action item's capture key, in timeline order.
fn keys(out: &CompileOutput) -> Vec<Hash> {
    out.timeline
        .entries
        .iter()
        .filter_map(|e| e.action.as_ref())
        .filter(|a| a.scene != "pause")
        .map(|a| a.capture_key)
        .collect()
}

const HEAD: &str = "\
---
teleprompt: 1
scene:
  terminal:
    plugin: mock
  other:
    plugin: mock
---

# A tour

";

/// Two paragraphs, each with the same tape under it.
fn twice(first: &str, second: &str) -> String {
    format!(
        "{HEAD}Move down the DAG list. {{#one}}\n\n\
         ```teleprompt scene=terminal\n{first}```\n\n\
         Move down the task list. {{#two}}\n\n\
         ```teleprompt scene=terminal\n{second}```\n"
    )
}

/// A one-second step, in the reference plugin's language.
const J: &str = "wait 1000ms\n";

/// The bug this exists to stop. `shot_hash` is the hash of a tape and
/// nothing else, so two blocks with the same steps collide — and the
/// second item would render the first item's picture. One is "move down
/// the DAG list", the other is "move down the task list".
#[test]
fn two_blocks_with_the_same_tape_do_not_share_a_picture() {
    let out = run(&twice(J, J));
    let hashes: Vec<Hash> = out
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.action.as_ref())
        .map(|a| a.shot_hash)
        .collect();

    assert_eq!(
        hashes[0], hashes[1],
        "the tapes really are identical, which is the whole difficulty"
    );
    let keys = keys(&out);
    assert_ne!(
        keys[0], keys[1],
        "but the screens are not: the second runs after the first"
    );
}

/// The chain's shape. Editing an item invalidates it and everything after
/// it in that session, and nothing before it.
#[test]
fn editing_a_shot_invalidates_it_and_what_follows_it() {
    let before = keys(&run(&twice(J, J)));
    let after = keys(&run(&twice(J, "wait 1500ms\n")));

    assert_eq!(before[0], after[0], "the item before the edit is untouched");
    assert_ne!(before[1], after[1], "the edited item is not");
}

#[test]
fn editing_the_first_beat_invalidates_every_shot_after_it() {
    let before = keys(&run(&twice(J, J)));
    let after = keys(&run(&twice("wait 700ms\n", J)));

    assert_ne!(before[0], after[0]);
    assert_ne!(
        before[1], after[1],
        "the second item's screen is what the first one left behind"
    );
}

/// A fresh session starts a fresh chain. Without a way to say so, a script
/// that quits a program and starts it again has no way to tell teleprompt
/// the screen was reset.
#[test]
fn a_named_session_starts_a_chain_of_its_own() {
    let joined = run(&twice(J, J));
    let split = run(&format!(
        "{HEAD}Move down the DAG list. {{#one}}\n\n\
         ```teleprompt scene=terminal\n{J}```\n\n\
         Start again. {{#two}}\n\n\
         ```teleprompt scene=terminal session=retry\n{J}```\n"
    ));

    assert_eq!(
        keys(&split)[0],
        keys(&joined)[0],
        "the first item is in neither session's debt"
    );
    assert_ne!(
        keys(&split)[1],
        keys(&joined)[1],
        "the second item no longer follows the first"
    );
    assert_eq!(
        keys(&split)[1],
        keys(&split)[0],
        "a session that opens with the same tape opens on the same screen, \
         so it is the same picture and the same clip"
    );
}

/// Two scenes are two screens. A item in one is not downstream of an item
/// in the other, however they interleave in the document.
#[test]
fn a_beat_does_not_follow_a_shot_in_another_scene() {
    let interleaved = run(&format!(
        "{HEAD}One. {{#one}}\n\n\
         ```teleprompt scene=terminal\n{J}```\n\n\
         Two. {{#two}}\n\n\
         ```teleprompt scene=other\n{J}```\n\n\
         Three. {{#three}}\n\n\
         ```teleprompt scene=terminal\n{J}```\n"
    ));
    let alone = keys(&run(&twice(J, J)));
    let keys = keys(&interleaved);

    assert_eq!(
        keys[2], alone[1],
        "the third item is the second thing to happen in its own scene, \
         and the item in between belongs to another screen"
    );

    // And the item in between opens a screen of its own, which — same
    // scene plugin, same settings, same tape — is the same picture as the one
    // the first item opened. That is deduplication, not a collision: two
    // identically configured scenes really do show the same thing.
    assert_eq!(keys[0], keys[1]);
}

/// Marks split one block into shots that share a session, so they chain
/// like blocks do — the second half of a block runs on what the first half
/// left on screen.
#[test]
fn the_shots_of_one_block_chain_like_blocks_do() {
    let out = run(&format!(
        "{HEAD}One. {{#one}}\n\n\
         ```teleprompt scene=terminal\n{J}mark\n{J}```\n"
    ));
    let keys = keys(&out);

    assert_eq!(keys.len(), 2, "two marks, two shots");
    assert_ne!(keys[0], keys[1]);
}

/// A pause is not a picture, so it is not in any chain: it holds whatever
/// is on screen. Chaining it would make every item after a pause depend on
/// how long the pause was.
#[test]
fn a_pause_does_not_join_the_chain() {
    let with = run(&format!(
        "{HEAD}One. {{#one}}\n\n\
         ```teleprompt scene=terminal\n{J}```\n\n\
         <!-- teleprompt: pause 2000ms -->\n\n\
         Two. {{#two}}\n\n\
         ```teleprompt scene=terminal\n{J}```\n"
    ));
    let without = run(&twice(J, J));

    assert_eq!(
        keys(&with),
        keys(&without),
        "a pause between two items changed what the second one shows"
    );
}

/// A clip is served from the cache on its key alone, so the key has to
/// name whatever drew it. It did not. `scene plugin` says `vhs`, and every
/// renderer teleprompt has ever pointed at a `vhs` scene also said `vhs`:
/// the name identifies the *scene language*, not the program that turns it
/// into pixels.
///
/// That is not hypothetical. Deleting the pty renderer left its clips in
/// the cache under exactly the keys its replacement asks for, and the next
/// build served twenty-four of them — a video assembled from a renderer
/// that no longer exists, reported as fully captured.
///
/// So the key carries a recipe, for the same reason the compose cache's
/// chunk key does: bump it and yesterday's pictures stop answering to
/// today's names.
#[test]
fn a_capture_key_names_the_recipe_that_recorded_it() {
    use teleprompt_compile::CAPTURE_RECIPE;
    use teleprompt_core::config::SceneConfig;

    let out = run(&format!(
        "{HEAD}One. {{#one}}\n\n\
         ```teleprompt scene=terminal\n{J}```\n"
    ));
    let action = out
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.action.as_ref())
        .find(|a| a.scene != "pause")
        .expect("the fixture has one action item");

    let settings = out
        .scenes
        .get(&action.scene)
        .map(SceneConfig::settings_fingerprint)
        .unwrap_or_default();
    let name = Hash::of_fields(&[
        CAPTURE_RECIPE,
        &action.plugin,
        &settings,
        &action.shot_hash.to_string(),
    ]);

    assert_eq!(
        action.capture_key,
        Hash::of_fields(&[&name.to_string()]),
        "the first item of a session should be chain(0) = H(name(0)), \
         with the recipe inside name(0)"
    );
}

/// The mock scene plugin, speaking for a scene whose shots carry no state — a
/// composition that draws the same frames whatever came before it.
struct Still;

impl teleprompt_plugin::scene::contract::SceneCompiler for Still {
    fn kind(&self) -> &'static str {
        "still"
    }
    fn validate(
        &self,
        src: &teleprompt_plugin::scene::contract::BlockSource,
    ) -> Result<teleprompt_plugin::scene::contract::Validated, Vec<teleprompt_core::Diagnostic>>
    {
        teleprompt_plugin::scene::mock::MockScene.validate(src)
    }
    fn shots(
        &self,
        v: &teleprompt_plugin::scene::contract::Validated,
        block_id: &BlockId,
    ) -> Result<Vec<teleprompt_plugin::scene::contract::Shot>, Vec<teleprompt_core::Diagnostic>>
    {
        teleprompt_plugin::scene::mock::MockScene.shots(v, block_id)
    }
    fn estimate(
        &self,
        shot: &teleprompt_plugin::scene::contract::Shot,
    ) -> teleprompt_plugin::scene::contract::Measured {
        teleprompt_plugin::scene::mock::MockScene.estimate(shot)
    }
    fn continues(&self) -> bool {
        false
    }
    fn inputs(&self, scene: &teleprompt_core::config::SceneConfig) -> Vec<std::path::PathBuf> {
        scene
            .settings
            .get("project")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .into_iter()
            .collect()
    }
}

fn stills(first: &str, second: &str) -> Vec<Hash> {
    let mut registry = SceneRegistry::with_builtins();
    registry.register(Box::new(Still));
    let src = twice(first, second).replace("plugin: mock\n  other", "plugin: still\n  other");
    keys(&run_with(&src, &registry))
}

/// A scene that does not continue its predecessor is named shot by shot.
/// Editing the first picture re-captures the first picture, and the one
/// after it — which draws the same frames whatever preceded it — is left
/// in the cache.
#[test]
fn a_scene_that_does_not_continue_names_each_shot_by_itself() {
    let before = stills(J, J);
    let after = stills("wait 2000ms\n", J);
    assert_ne!(before[0], after[0], "the edited shot is re-captured");
    assert_eq!(before[1], after[1], "the one after it is not");
}

/// The other side of the same fact: with nothing carried between shots,
/// two with the same source are the same picture, and share one clip.
#[test]
fn identical_shots_of_a_scene_that_does_not_continue_share_a_clip() {
    let keys = stills(J, J);
    assert_eq!(keys[0], keys[1]);
}

/// Keys for two shots in a scene drawn from `project`.
fn drawn_from(project: &std::path::Path) -> Vec<Hash> {
    let mut registry = SceneRegistry::with_builtins();
    registry.register(Box::new(Still));
    let src = twice(J, J).replace(
        "plugin: mock\n  other",
        &format!("plugin: still\n    project: {}\n  other", project.display()),
    );
    keys(&run_with(&src, &registry))
}

/// A scene drawn by a project of its own is named by what the project
/// holds: editing a component re-captures the shots that draw with it,
/// and a file the scene does not read — its dependencies — does not.
#[test]
fn editing_a_file_the_scene_draws_from_re_captures_it() {
    let dir = teleprompt_testkit::test_dir("inputs");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("node_modules")).unwrap();
    std::fs::write(dir.join("src/Title.tsx"), "red").unwrap();

    let before = drawn_from(&dir);
    std::fs::write(dir.join("node_modules/dep.js"), "ignored").unwrap();
    assert_eq!(before, drawn_from(&dir), "node_modules is not an input");

    std::fs::write(dir.join("src/Title.tsx"), "blue").unwrap();
    let after = drawn_from(&dir);
    assert_ne!(before[0], after[0], "the edit re-captures the first shot");
    assert_ne!(before[1], after[1], "and the second");
}

/// A stateless scene plugin each of whose shots shows one file of its own: the
/// mock's `wait 1000ms` shows `one`, anything else `two`.
struct Pictured;

impl teleprompt_plugin::scene::contract::SceneCompiler for Pictured {
    fn kind(&self) -> &'static str {
        "pictured"
    }
    fn validate(
        &self,
        src: &teleprompt_plugin::scene::contract::BlockSource,
    ) -> Result<teleprompt_plugin::scene::contract::Validated, Vec<teleprompt_core::Diagnostic>>
    {
        teleprompt_plugin::scene::mock::MockScene.validate(src)
    }
    fn shots(
        &self,
        v: &teleprompt_plugin::scene::contract::Validated,
        block_id: &BlockId,
    ) -> Result<Vec<teleprompt_plugin::scene::contract::Shot>, Vec<teleprompt_core::Diagnostic>>
    {
        teleprompt_plugin::scene::mock::MockScene.shots(v, block_id)
    }
    fn estimate(
        &self,
        shot: &teleprompt_plugin::scene::contract::Shot,
    ) -> teleprompt_plugin::scene::contract::Measured {
        teleprompt_plugin::scene::mock::MockScene.estimate(shot)
    }
    fn continues(&self) -> bool {
        false
    }
    fn shot_inputs(
        &self,
        scene: &teleprompt_core::config::SceneConfig,
        source: &str,
    ) -> Vec<std::path::PathBuf> {
        let dir = scene.settings["project"].as_str().unwrap().to_string();
        let file = if source.contains("1000ms") {
            "one"
        } else {
            "two"
        };
        vec![std::path::Path::new(&dir).join(file)]
    }
}

fn pictured(project: &std::path::Path) -> Vec<Hash> {
    let mut registry = SceneRegistry::with_builtins();
    registry.register(Box::new(Pictured));
    let src = twice(J, "wait 2000ms\n").replace(
        "plugin: mock\n  other",
        &format!(
            "plugin: pictured\n    project: {}\n  other",
            project.display()
        ),
    );
    keys(&run_with(&src, &registry))
}

/// A shot is named by the file it shows, and only by that one: replacing
/// it re-captures the shots that show it, and nothing else.
#[test]
fn replacing_a_file_re_captures_only_the_shots_that_show_it() {
    let dir = teleprompt_testkit::test_dir("shot-inputs");
    std::fs::write(dir.join("one"), "red").unwrap();
    std::fs::write(dir.join("two"), "green").unwrap();

    let before = pictured(&dir);
    std::fs::write(dir.join("one"), "blue").unwrap();
    let after = pictured(&dir);
    assert_ne!(before[0], after[0], "the shot showing `one` is re-captured");
    assert_eq!(before[1], after[1], "the shot showing `two` is not");
}
