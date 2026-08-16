use teleprompt_voice_kokoro::KokoroConfig;

fn yaml(s: &str) -> serde_yaml::Value {
    serde_yaml::from_str(s).unwrap()
}

#[test]
fn defaults_match_the_spec() {
    let c = KokoroConfig::default();
    assert_eq!(c.base_url, "http://localhost:8880");
    assert_eq!(c.timeout_ms, 30_000);
    assert_eq!(c.concurrency, 4);
    assert_eq!(c.model, "kokoro");
}

#[test]
fn partial_settings_keep_the_other_defaults() {
    let c = KokoroConfig::from_value(&yaml("timeout_ms: 5000")).unwrap();
    assert_eq!(c.timeout_ms, 5_000);
    assert_eq!(c.base_url, "http://localhost:8880");
    assert_eq!(c.concurrency, 4);
}

#[test]
fn a_trailing_slash_on_base_url_does_not_produce_a_double_slash() {
    let c = KokoroConfig::from_value(&yaml("base_url: \"http://x:8880/\"")).unwrap();
    assert_eq!(c.base_url, "http://x:8880");
}

#[test]
fn an_unknown_setting_is_rejected_rather_than_ignored() {
    // A typo in a backend setting is otherwise invisible: the run succeeds
    // with the default and the author never learns their value did nothing.
    let err = KokoroConfig::from_value(&yaml("timeuot_ms: 5000")).unwrap_err();
    assert!(err.contains("timeuot_ms"), "{err}");
}

#[test]
fn zero_concurrency_is_rejected() {
    let err = KokoroConfig::from_value(&yaml("concurrency: 0")).unwrap_err();
    assert!(err.contains("concurrency"), "{err}");
}

// The load-bearing one. `teleprompt_cache::key` is built from `backend_id`,
// `backend_version` and the SynthRequest — text, locale, voice, speed. A
// Kokoro server's audio also depends on which server it is and which model
// it loaded, and neither reaches the key except through this string.
#[test]
fn the_version_string_separates_hosts_and_models() {
    let a = KokoroConfig::from_value(&yaml("base_url: \"http://localhost:8880\"")).unwrap();
    let b = KokoroConfig::from_value(&yaml("base_url: \"http://gpu-box:8880\"")).unwrap();
    let c = KokoroConfig::from_value(&yaml(
        "base_url: \"http://localhost:8880\"\nmodel: kokoro-v1_1",
    ))
    .unwrap();

    assert_ne!(
        a.version_string(),
        b.version_string(),
        "host must be in the key"
    );
    assert_ne!(
        a.version_string(),
        c.version_string(),
        "model must be in the key"
    );
}

#[test]
fn settings_that_cannot_change_the_audio_stay_out_of_the_version() {
    // Otherwise every timeout or concurrency retune invalidates the whole
    // cache and re-synthesizes a project that did not change.
    let a = KokoroConfig::from_value(&yaml("timeout_ms: 30000\nconcurrency: 4")).unwrap();
    let b = KokoroConfig::from_value(&yaml("timeout_ms: 1000\nconcurrency: 16")).unwrap();
    assert_eq!(a.version_string(), b.version_string());
}

// `version_string` takes the host by splitting on "://", not by parsing a
// URL, so userinfo in `base_url` rides along unexamined. That is safe for
// key correctness only because it cannot go the wrong way: pins the "no
// collision" half of that claim, not merely today's exact string.
#[test]
fn a_userinfo_bearing_base_url_stays_distinct_rather_than_colliding() {
    let plain = KokoroConfig::from_value(&yaml("base_url: \"http://host:8880\"")).unwrap();
    let with_userinfo =
        KokoroConfig::from_value(&yaml("base_url: \"http://user:pass@host:8880\"")).unwrap();
    assert_ne!(
        plain.version_string(),
        with_userinfo.version_string(),
        "a userinfo-bearing base_url must never collide with the plain-host version"
    );
}
