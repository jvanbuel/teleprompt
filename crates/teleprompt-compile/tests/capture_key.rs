//! What names a beat's picture.
//!
//! A scene is a session: the beats of a walkthrough continue one another,
//! and the screen at beat *N* is the accumulation of beats 1..*N*. So a
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
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::resolve;
use teleprompt_core::Hash;
use teleprompt_scene::SceneRegistry;
use teleprompt_voice_null::WpmEstimator;

fn run(src: &str) -> CompileOutput {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let cache =
        VoiceCache::new(std::env::temp_dir().join(format!("tp-chain-{}-{n}", std::process::id())));
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
    };
    let mut parsed = parse_script(src).expect("fixture parses");
    assign_ids(&mut parsed);
    let program = resolve(
        &parsed,
        "tour.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect("fixture resolves");
    compile(
        &program,
        &SceneRegistry::with_builtins(),
        &ctx,
        Path::new("."),
        "0.1.0",
    )
    .expect("fixture compiles")
}

/// Every action beat's capture key, in timeline order.
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
    adapter: mock
  other:
    adapter: mock
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

/// A one-second step, in the reference adapter's language.
const J: &str = "wait 1000ms\n";

/// The bug this exists to stop. `span_hash` is the hash of a tape and
/// nothing else, so two blocks with the same steps collide — and the
/// second beat would render the first beat's picture. One is "move down
/// the DAG list", the other is "move down the task list".
#[test]
fn two_blocks_with_the_same_tape_do_not_share_a_picture() {
    let out = run(&twice(J, J));
    let hashes: Vec<Hash> = out
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.action.as_ref())
        .map(|a| a.span_hash)
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

/// The chain's shape. Editing a beat invalidates it and everything after
/// it in that session, and nothing before it.
#[test]
fn editing_a_beat_invalidates_it_and_what_follows_it() {
    let before = keys(&run(&twice(J, J)));
    let after = keys(&run(&twice(J, "wait 1500ms\n")));

    assert_eq!(before[0], after[0], "the beat before the edit is untouched");
    assert_ne!(before[1], after[1], "the edited beat is not");
}

#[test]
fn editing_the_first_beat_invalidates_every_beat_after_it() {
    let before = keys(&run(&twice(J, J)));
    let after = keys(&run(&twice("wait 700ms\n", J)));

    assert_ne!(before[0], after[0]);
    assert_ne!(
        before[1], after[1],
        "the second beat's screen is what the first one left behind"
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
        "the first beat is in neither session's debt"
    );
    assert_ne!(
        keys(&split)[1],
        keys(&joined)[1],
        "the second beat no longer follows the first"
    );
    assert_eq!(
        keys(&split)[1],
        keys(&split)[0],
        "a session that opens with the same tape opens on the same screen, \
         so it is the same picture and the same clip"
    );
}

/// Two scenes are two screens. A beat in one is not downstream of a beat
/// in the other, however they interleave in the document.
#[test]
fn a_beat_does_not_follow_a_beat_in_another_scene() {
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
        "the third beat is the second thing to happen in its own scene, \
         and the beat in between belongs to another screen"
    );

    // And the beat in between opens a screen of its own, which — same
    // adapter, same settings, same tape — is the same picture as the one
    // the first beat opened. That is deduplication, not a collision: two
    // identically configured scenes really do show the same thing.
    assert_eq!(keys[0], keys[1]);
}

/// Marks split one block into spans that share a session, so they chain
/// like blocks do — the second half of a block runs on what the first half
/// left on screen.
#[test]
fn the_spans_of_one_block_chain_like_blocks_do() {
    let out = run(&format!(
        "{HEAD}One. {{#one}}\n\n\
         ```teleprompt scene=terminal\n{J}mark\n{J}```\n"
    ));
    let keys = keys(&out);

    assert_eq!(keys.len(), 2, "two marks, two spans");
    assert_ne!(keys[0], keys[1]);
}

/// A pause is not a picture, so it is not in any chain: it holds whatever
/// is on screen. Chaining it would make every beat after a pause depend on
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
        "a pause between two beats changed what the second one shows"
    );
}

/// A clip is served from the cache on its key alone, so the key has to
/// name whatever drew it. It did not. `adapter` says `vhs`, and every
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
        .expect("the fixture has one action beat");

    let settings = out
        .scenes
        .get(&action.scene)
        .map(SceneConfig::settings_fingerprint)
        .unwrap_or_default();
    let name = Hash::of_fields(&[
        CAPTURE_RECIPE,
        &action.adapter,
        &settings,
        &action.span_hash.to_string(),
    ]);

    assert_eq!(
        action.capture_key,
        Hash::of_fields(&[&name.to_string()]),
        "the first beat of a session should be chain(0) = H(name(0)), \
         with the recipe inside name(0)"
    );
}
