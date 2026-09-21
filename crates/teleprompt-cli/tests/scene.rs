//! The composition root: which adapters a built `teleprompt` can actually
//! reach.
//!
//! `SceneRegistry::with_builtins()` carries only the adapters inside
//! `teleprompt-scene`, so an adapter living in its own crate is reachable
//! only if something adds it. That something is `scene::scenes()`, and these
//! tests are what catch a command compiling against the narrower registry —
//! which fails `scene=terminal` with "no adapter `vhs` is available" on a
//! build that ships one.

use teleprompt_cli::scene::scenes;
use teleprompt_scene::SceneCompiler;

#[test]
fn the_registry_serves_every_adapter_this_build_ships() {
    let r = scenes();

    assert_eq!(r.get("mock").map(SceneCompiler::kind), Some("mock"));
    assert_eq!(r.get("vhs").map(SceneCompiler::kind), Some("vhs"));
    assert_eq!(r.available(), vec!["mock", "vhs"]);
}

#[test]
fn an_unknown_adapter_is_not_served() {
    assert!(scenes().get("playwright").is_none());
}

/// The other half of `stretch-action`: the scheduler decides the action
/// should fill the sentence over it, and the tape that gets captured is
/// re-timed to last that long. Without this the number moves and the tape
/// does not — a capture runs at the authored pace and the rest of the slot
/// is a frozen frame.
#[tokio::test]
async fn a_stretched_span_is_published_re_timed_to_its_scheduled_length() {
    use teleprompt_cli::project::Project;
    use teleprompt_scene::Measured;

    let dir = std::env::temp_dir().join(format!("tp-stretch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/stretch.md");
    std::fs::write(
        &script,
        "---\nteleprompt: 1\nscene:\n  terminal:\n    adapter: vhs\n---\n\n\
         # Stretching\n\n\
         This paragraph runs far longer than the two seconds of tape beneath it, \
         which is the whole point of asking the action to stretch: it should fill \
         the sentence rather than finish early and leave the picture sitting \
         still. {#long}\n\n\
         ```teleprompt scene=terminal policy=stretch-action\n\
         Set TypingSpeed 50ms\n\
         Type \"ls -la\"\n\
         Enter\n\
         Sleep 1s\n\
         ```\n",
    )
    .unwrap();

    let project = Project::discover(&dir).unwrap();
    let (compiled, _) =
        teleprompt_cli::cmd::check::compile_script(&project, &script, "en").expect("it compiles");

    let entry = compiled
        .timeline
        .entries
        .iter()
        .find_map(|e| e.action.as_ref())
        .expect("the beat has an action");
    let published = compiled
        .spans
        .iter()
        .find(|s| s.id == entry.span)
        .expect("its source is published");

    let span = teleprompt_scene::Span {
        id: published.id.clone(),
        source: published.source.clone(),
        hash: entry.span_hash,
        index: 0,
    };
    let registry = scenes();
    let adapter = registry.get("vhs").expect("this build ships vhs");
    assert_eq!(
        adapter.estimate(&span),
        Measured::Exact(entry.duration_ms),
        "the published tape should last exactly its slot:\n{}",
        published.source
    );
    assert!(
        entry.duration_ms > 2_000,
        "the slot really was stretched: {}ms",
        entry.duration_ms
    );
}
