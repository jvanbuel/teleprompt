use teleprompt_cli::cmd::new::scaffold;
use teleprompt_core::SpanMs;
use teleprompt_scene::SceneRegistry;

#[test]
fn scaffold_writes_a_runnable_project() {
    let dir = tempdir();
    let written = scaffold(&dir).unwrap();
    let names: Vec<String> = written
        .iter()
        .map(|p| p.strip_prefix(&dir).unwrap().display().to_string())
        .collect();
    assert!(names.contains(&"teleprompt.toml".to_string()));
    assert!(names.contains(&"scripts/demo.md".to_string()));
    assert!(names.contains(&".gitignore".to_string()));
}

#[test]
fn the_scaffolded_script_compiles() {
    use teleprompt_cache::VoiceCache;
    use teleprompt_compile::{compile, VoiceContext};
    use teleprompt_core::config::PartialConfig;
    use teleprompt_core::parse::parse_script;
    use teleprompt_core::program::resolve;
    use teleprompt_voice::WpmEstimator;

    let dir = tempdir();
    scaffold(&dir).unwrap();
    let src = std::fs::read_to_string(dir.join("scripts/demo.md")).unwrap();

    let script = parse_script(&src).expect("scaffolded script must parse");
    let program = resolve(
        &script,
        "demo.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect("scaffolded script must resolve");
    let cache = VoiceCache::new(dir.join(".teleprompt/cache"));
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
        takes: &teleprompt_voice::takes::Takes::default(),
    };
    let out = compile(
        &program,
        &SceneRegistry::with_builtins(),
        &ctx,
        &dir,
        "0.1.0",
    )
    .expect("scaffolded script must compile");
    assert!(out.timeline.duration_ms > SpanMs::of(0));
}

#[test]
fn scaffold_refuses_to_clobber_an_existing_project() {
    let dir = tempdir();
    scaffold(&dir).unwrap();
    let err = scaffold(&dir).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
}

#[test]
fn the_gitignore_excludes_caches_and_build_output_but_not_timelines() {
    let dir = tempdir();
    scaffold(&dir).unwrap();
    let ignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert!(ignore.contains(".teleprompt/cache/"));
    assert!(ignore.contains("build/"));
    assert!(!ignore.contains("timelines/"));
    // Takes are source: nothing can record them again.
    assert!(!ignore.contains("takes"), "{ignore}");
}

fn tempdir() -> teleprompt_testkit::TestDir {
    teleprompt_testkit::test_dir("test")
}
