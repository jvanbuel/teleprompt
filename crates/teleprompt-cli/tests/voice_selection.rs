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
    let err = registry_for(&settings("concurrency: 0")).unwrap_err();
    assert!(err.contains("concurrency"), "{err}");
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
