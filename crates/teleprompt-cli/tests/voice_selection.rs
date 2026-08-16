use std::collections::BTreeMap;

use teleprompt_cli::voice::{registry, registry_for};

fn settings(yaml: &str) -> BTreeMap<String, serde_yaml::Value> {
    let mut m = BTreeMap::new();
    m.insert("kokoro".to_string(), serde_yaml::from_str(yaml).unwrap());
    m
}

#[test]
fn kokoro_is_registered_by_default() {
    let r = registry();
    assert_eq!(r.available(), vec!["kokoro", "null"]);
}

#[test]
fn kokoro_takes_its_settings_from_the_backends_map() {
    let r = registry_for(&settings("base_url: \"http://gpu-box:8880\"")).unwrap();
    let k = r.get("kokoro").unwrap();
    // The version string is what keys the cache, so this is the observable
    // that proves the settings reached the backend rather than the default.
    assert!(
        k.capabilities().version.contains("gpu-box"),
        "{}",
        k.capabilities().version
    );
}

#[test]
fn a_bad_backend_setting_is_reported_not_swallowed() {
    // `VoiceRegistry` deliberately has no `Debug` impl (see
    // `teleprompt-voice`), so `unwrap_err` — which would need one to print
    // an `Ok` value it doesn't get — is not available here; match instead.
    match registry_for(&settings("concurrency: 0")) {
        Ok(_) => panic!("expected an error"),
        Err(e) => assert!(e.contains("concurrency"), "{e}"),
    }
}

#[test]
fn settings_for_a_backend_this_build_lacks_are_ignored_here() {
    // Reporting an unknown backend is `resolve`'s job, and only when the
    // script actually selects it. A project carrying settings for a backend
    // it does not currently use must still build.
    let mut m = BTreeMap::new();
    m.insert(
        "elevenlabs".to_string(),
        serde_yaml::from_str("profile: jan").unwrap(),
    );
    assert!(registry_for(&m).is_ok());
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "teleprompt-voice-selection-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

const SCRIPT_WITH_FRONT_MATTER_BACKENDS: &str = "\
---
backends:
  kokoro:
    base_url: \"http://gpu-box:8880\"
---

# Quick start

Every video in this repository is built from a script you can read.
";

/// A script's own front matter can carry `backends:` too — `Config`'s doc
/// comment says so, and `resolve` genuinely merges it into
/// `program.config.backends`. But the registry `compile_script` builds is
/// constructed from the *project's* `backends:` alone, before this script
/// is ever read, so the override the merge computed is never seen by
/// anything that could act on it. That must be named, not dropped.
#[test]
fn a_front_matter_backends_override_is_reported_not_silently_dropped() {
    let dir = tempdir("front-matter-backends");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(
        dir.join("scripts/test.md"),
        SCRIPT_WITH_FRONT_MATTER_BACKENDS,
    )
    .unwrap();
    let project = teleprompt_cli::project::Project::discover(&dir).unwrap();
    let script = dir.join("scripts/test.md");

    let warnings =
        teleprompt_cli::cmd::check::run_check(&project, &script, "en").unwrap_or_else(|e| {
            panic!("a dropped front-matter override must not fail the compile: {e:?}")
        });

    let msg = warnings
        .iter()
        .find(|w| w.contains("kokoro"))
        .unwrap_or_else(|| panic!("expected a warning naming kokoro, got {warnings:?}"));
    assert!(msg.contains("teleprompt.toml"), "{msg}");
    assert!(
        msg.contains("no effect") || msg.contains("not applied"),
        "{msg}"
    );
}

/// The negative case: a project whose `teleprompt.toml` already sets the
/// same `backends.kokoro` value the script's front matter repeats must not
/// warn — the merge genuinely agrees with what the registry was built
/// from, so there is nothing dropped to report.
#[test]
fn a_front_matter_backends_block_matching_the_project_is_not_a_warning() {
    let dir = tempdir("front-matter-backends-match");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(
        dir.join("teleprompt.toml"),
        "[backends.kokoro]\nbase_url = \"http://gpu-box:8880\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("scripts/test.md"),
        SCRIPT_WITH_FRONT_MATTER_BACKENDS,
    )
    .unwrap();
    let project = teleprompt_cli::project::Project::discover(&dir).unwrap();
    let script = dir.join("scripts/test.md");

    let warnings = teleprompt_cli::cmd::check::run_check(&project, &script, "en").unwrap();
    assert!(
        !warnings.iter().any(|w| w.contains("kokoro")),
        "{warnings:?}"
    );
}
