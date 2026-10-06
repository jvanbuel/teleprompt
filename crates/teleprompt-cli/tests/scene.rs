//! The composition root: which scene plugins a built `teleprompt` can actually
//! reach.
//!
//! `ScenePlugins::mock()` carries only the mock, so a scene plugin living
//! in its own crate is reachable only if something adds it. That something
//! is `scene::registry().scenes`, and these tests are what catch a command
//! compiling against the narrower set — which fails `scene=terminal`,
//! saying the scene plugin `vhs` is not available, on a build that ships
//! one.

use teleprompt_core::SpanMs;
use teleprompt_registry::registry;

#[test]
fn the_registry_serves_every_plugin_this_build_ships() {
    let r = registry().scenes;
    for name in [
        "mock",
        "vhs",
        "playwright",
        "remotion",
        "slidev",
        "asciinema",
    ] {
        assert_eq!(r.get(name).map(|p| p.scene().kind()), Some(name));
    }
    let mut names = r.names();
    names.sort_unstable();
    assert_eq!(
        names,
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
fn an_unknown_plugin_is_not_served() {
    assert!(registry().scenes.get("selenium").is_none());
}

/// The other half of `fit-action`: the scheduler decides the action
/// should fill the sentence over it, and the tape that gets captured is
/// re-timed to last that long. Without this the number moves and the tape
/// does not — a capture runs at the authored pace and the rest of the slot
/// is a frozen frame.
#[tokio::test]
async fn a_stretched_shot_is_published_re_timed_to_its_scheduled_length() {
    use teleprompt::project::Project;
    use teleprompt_plugin::scene::Measured;

    let dir = teleprompt_testkit::test_dir("stretch");
    teleprompt::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/stretch.md");
    std::fs::write(
        &script,
        "---\nteleprompt: 1\nscene:\n  terminal:\n    plugin: vhs\n---\n\n\
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

    let project = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let compiled = project
        .script(&script, "en")
        .compile()
        .expect("it compiles")
        .output;

    let entry = compiled
        .timeline
        .entries
        .iter()
        .find_map(|e| e.action.as_ref())
        .expect("the item has an action");
    let published = compiled
        .shots
        .get(&entry.shot)
        .expect("its source is published");

    // Split again, as a fresh block: what the tape says it lasts.
    let plugin = registry()
        .scenes
        .get("vhs")
        .expect("this build ships vhs")
        .scene();
    let again = teleprompt_plugin::scene::Validated {
        scene: published.scene.clone(),
        body: published.source.clone(),
    };
    let shots = plugin
        .shots(&again, &teleprompt_core::BlockId::new("again"))
        .expect("the published tape splits");
    assert_eq!(
        shots[0].length,
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
    teleprompt::new::scaffold(&dir).unwrap();
    std::fs::write(
        dir.join("teleprompt.toml"),
        "[locales]\nsource = \"en\"\n\n[scene.media]\ndir = \"pictures\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("pictures")).unwrap();
    std::fs::write(
        dir.join("scripts/pics.md"),
        "---\nteleprompt: 1\n---\n\n# Pictures\n\nA picture. {#pic}\n\n```teleprompt scene=media\nimage src=a.png\n```\n",
    )
    .unwrap();
    let key = || {
        let project =
            teleprompt::project::Project::discover(&dir, teleprompt_registry::registry()).unwrap();
        let compiled = project
            .script(dir.join("scripts/pics.md"), "en")
            .compile()
            .unwrap_or_else(|e| panic!("{e:?}"));
        compiled.output.timeline.entries[0]
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
