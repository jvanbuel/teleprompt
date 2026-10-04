//! The composition root: which scene plugins a built `teleprompt` can actually
//! reach.
//!
//! `SceneRegistry::with_builtins()` carries only the scene plugins inside
//! `teleprompt_plugin::scene`, so a scene plugin living in its own crate is reachable
//! only if something adds it. That something is `scene::scenes()`, and these
//! tests are what catch a command compiling against the narrower registry —
//! which fails `scene=terminal`, saying the scene plugin `vhs` is not available, on a
//! build that ships one.

use teleprompt_cli::scene::scenes;
use teleprompt_core::SpanMs;
use teleprompt_plugin::scene::SceneCompiler;

#[test]
fn the_registry_serves_every_adapter_this_build_ships() {
    let r = scenes();

    assert_eq!(r.get("mock").map(SceneCompiler::kind), Some("mock"));
    assert_eq!(r.get("vhs").map(SceneCompiler::kind), Some("vhs"));
    assert_eq!(
        r.get("playwright").map(SceneCompiler::kind),
        Some("playwright")
    );
    assert_eq!(r.get("remotion").map(SceneCompiler::kind), Some("remotion"));
    assert_eq!(r.get("slidev").map(SceneCompiler::kind), Some("slidev"));
    assert_eq!(
        r.get("asciinema").map(SceneCompiler::kind),
        Some("asciinema")
    );
    assert_eq!(
        r.available(),
        vec![
            "asciinema",
            "macos",
            "media",
            "mock",
            "playwright",
            "remotion",
            "slidev",
            "vhs",
            "x11"
        ]
    );
}

#[test]
fn an_unknown_adapter_is_not_served() {
    assert!(scenes().get("selenium").is_none());
}

/// The other half of `fit-action`: the scheduler decides the action
/// should fill the sentence over it, and the tape that gets captured is
/// re-timed to last that long. Without this the number moves and the tape
/// does not — a capture runs at the authored pace and the rest of the slot
/// is a frozen frame.
#[tokio::test]
async fn a_stretched_shot_is_published_re_timed_to_its_scheduled_length() {
    use teleprompt_cli::project::Project;
    use teleprompt_plugin::scene::Measured;

    let dir = teleprompt_testkit::test_dir("stretch");
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
         ```teleprompt scene=terminal policy=fit-action\n\
         Set TypingSpeed 50ms\n\
         Type \"ls -la\"\n\
         Enter\n\
         Sleep 1s\n\
         ```\n",
    )
    .unwrap();

    let project = Project::discover(&dir).unwrap();
    let (compiled, _) = project.compile(&script, "en").expect("it compiles");

    let entry = compiled
        .timeline
        .entries
        .iter()
        .find_map(|e| e.action.as_ref())
        .expect("the item has an action");
    let published = compiled
        .shots
        .iter()
        .find(|s| s.id == entry.shot)
        .expect("its source is published");

    let shot = teleprompt_plugin::scene::Shot {
        id: published.id.clone(),
        source: published.source.clone(),
        hash: entry.shot_hash,
        index: 0,
    };
    let registry = scenes();
    let adapter = registry.get("vhs").expect("this build ships vhs");
    assert_eq!(
        adapter.estimate(&shot),
        Measured::Exact(entry.duration_ms.ms()),
        "the published tape should last exactly its slot:\n{}",
        published.source
    );
    assert!(
        entry.duration_ms > SpanMs::of(2_000),
        "the slot really was stretched: {}ms",
        entry.duration_ms.ms()
    );
}

/// A path in a scene's settings is the project's, wherever teleprompt runs
/// (these tests run in the crate's directory): the image a shot shows is
/// found, so replacing it changes the shot's key.
#[test]
fn a_scene_reads_its_files_from_the_project() {
    let dir = teleprompt_testkit::test_dir("scene-paths");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(
        dir.join("teleprompt.toml"),
        "[locales]\nsource = \"en\"\ntargets = []\n\n[scene.media]\ndir = \"pictures\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("pictures")).unwrap();
    std::fs::write(
        dir.join("scripts/pics.md"),
        "---\nteleprompt: 1\n---\n\n# Pictures\n\nA picture. {#pic}\n\n```teleprompt scene=media\nimage src=a.png\n```\n",
    )
    .unwrap();
    let key = || {
        let project = teleprompt_cli::project::Project::discover(&dir).unwrap();
        let compiled = project
            .compile(&dir.join("scripts/pics.md"), "en")
            .unwrap_or_else(|e| panic!("{e:?}"));
        compiled.0.timeline.entries[0]
            .action
            .as_ref()
            .unwrap()
            .capture_key
    };
    std::fs::write(dir.join("pictures/a.png"), b"one").unwrap();
    let first = key();
    std::fs::write(dir.join("pictures/a.png"), b"two").unwrap();
    assert_ne!(first, key(), "the image was not read from the project");
}
