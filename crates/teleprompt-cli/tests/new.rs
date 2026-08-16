use teleprompt_cli::cmd::doctor::doctor_report;
use teleprompt_cli::cmd::new::scaffold;
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
    use teleprompt_core::ident::assign_ids;
    use teleprompt_core::parse::parse_script;
    use teleprompt_core::program::resolve;
    use teleprompt_voice_null::WpmEstimator;

    let dir = tempdir();
    scaffold(&dir).unwrap();
    let src = std::fs::read_to_string(dir.join("scripts/demo.md")).unwrap();

    let mut script = parse_script(&src).expect("scaffolded script must parse");
    assign_ids(&mut script);
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
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
    };
    let out = compile(
        &program,
        &SceneRegistry::with_builtins(),
        &ctx,
        &dir,
        "0.1.0",
    )
    .expect("scaffolded script must compile");
    assert!(out.timeline.duration_ms > 0);
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
}

#[tokio::test]
async fn doctor_reports_available_adapters_and_backends() {
    let r = doctor_report(&SceneRegistry::with_builtins()).await;
    assert!(r.adapters.contains(&"mock".to_string()));
    assert!(r.voice_backends.contains(&"null".to_string()));
    assert!(r.voice_backends.contains(&"kokoro".to_string()));
}

#[tokio::test]
async fn doctor_notes_that_rendering_is_out_of_scope_rather_than_failing_on_ffmpeg() {
    let r = doctor_report(&SceneRegistry::with_builtins()).await;
    assert!(r.notes.iter().any(|n| n.contains("M0")));
    assert!(r.ok, "a missing ffmpeg must not make doctor fail in M0");
}

fn tempdir() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "teleprompt-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}
