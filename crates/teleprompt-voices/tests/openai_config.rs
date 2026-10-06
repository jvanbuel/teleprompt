use teleprompt_voices::openai::OpenAiConfig;

fn yaml(s: &str) -> serde_yaml::Value {
    serde_yaml::from_str(s).unwrap()
}

#[test]
fn defaults_match_the_spec() {
    let c = OpenAiConfig::kokoro();
    assert_eq!(c.base_url, "http://localhost:8880");
    assert_eq!(c.timeout_ms, 30_000);
    assert_eq!(c.concurrency, 4);
    assert_eq!(c.model, "kokoro");
}

#[test]
fn partial_settings_keep_the_other_defaults() {
    let c = OpenAiConfig::kokoro()
        .with(&yaml("timeout_ms: 5000"))
        .unwrap();
    assert_eq!(c.timeout_ms, 5_000);
    assert_eq!(c.base_url, "http://localhost:8880");
    assert_eq!(c.concurrency, 4);
}

#[test]
fn a_trailing_slash_on_base_url_does_not_produce_a_double_slash() {
    let c = OpenAiConfig::kokoro()
        .with(&yaml("base_url: \"http://x:8880/\""))
        .unwrap();
    assert_eq!(c.base_url, "http://x:8880");
}

#[test]
fn an_unknown_setting_is_rejected_rather_than_ignored() {
    // A typo in a backend setting is otherwise invisible: the run succeeds
    // with the default and the author never learns their value did nothing.
    let err = OpenAiConfig::kokoro()
        .with(&yaml("timeuot_ms: 5000"))
        .unwrap_err();
    assert!(err.contains("timeuot_ms"), "{err}");
}

#[test]
fn zero_concurrency_is_rejected() {
    let err = OpenAiConfig::kokoro()
        .with(&yaml("concurrency: 0"))
        .unwrap_err();
    assert!(err.contains("concurrency"), "{err}");
}

// The load-bearing one. `teleprompt_voice::cache::key` is built from `backend_id`,
// `backend_version` and the SynthRequest — text, locale, voice, speed. What
// a Kokoro server produces depends on the model it loaded, and that reaches
// the key only through this string.
//
// The *machine* deliberately does not. A cache keyed on the server's
// address is a cache that never crosses machines: the same model behind
// `localhost` and behind `gpu-box` produces identical audio and two
// different keys, so CI and every second contributor start cold. Worse,
// `localhost` and `127.0.0.1` do it on one machine.
#[test]
fn the_version_string_is_the_model_and_not_the_machine() {
    let local = OpenAiConfig::kokoro()
        .with(&yaml("base_url: \"http://localhost:8880\""))
        .unwrap();
    let loopback = OpenAiConfig::kokoro()
        .with(&yaml("base_url: \"http://127.0.0.1:8880\""))
        .unwrap();
    let remote = OpenAiConfig::kokoro()
        .with(&yaml("base_url: \"http://gpu-box:8880\""))
        .unwrap();
    let other_model = OpenAiConfig::kokoro()
        .with(&yaml(
            "base_url: \"http://localhost:8880\"\nmodel: kokoro-v1_1",
        ))
        .unwrap();

    assert_eq!(
        local.version_string(),
        loopback.version_string(),
        "two spellings of the same machine are the same cache"
    );
    assert_eq!(
        local.version_string(),
        remote.version_string(),
        "and so is somebody else's machine running the same model"
    );
    assert_ne!(
        local.version_string(),
        other_model.version_string(),
        "the model still has to be in the key"
    );
}

// The trade this makes, stated as a test so it cannot be forgotten: two
// servers serving different weights under one model name now collide. That
// is why `setup` reports the model alongside the address — the mismatch
// has to be visible somewhere, and the cache key is no longer the place.
#[test]
fn nothing_about_the_server_address_survives_into_the_version() {
    let plain = OpenAiConfig::kokoro()
        .with(&yaml("base_url: \"http://host:8880\""))
        .unwrap();
    let with_userinfo = OpenAiConfig::kokoro()
        .with(&yaml("base_url: \"http://user:pass@host:8880\""))
        .unwrap();
    let elsewhere = OpenAiConfig::kokoro()
        .with(&yaml("base_url: \"https://kokoro.internal/api\""))
        .unwrap();

    assert_eq!(plain.version_string(), with_userinfo.version_string());
    assert_eq!(plain.version_string(), elsewhere.version_string());
    assert!(
        !plain.version_string().contains("host"),
        "the address is not in it at all: {}",
        plain.version_string()
    );
}

#[test]
fn settings_that_cannot_change_the_audio_stay_out_of_the_version() {
    // Otherwise every timeout or concurrency retune invalidates the whole
    // cache and re-synthesizes a project that did not change.
    let a = OpenAiConfig::kokoro()
        .with(&yaml("timeout_ms: 30000\nconcurrency: 4"))
        .unwrap();
    let b = OpenAiConfig::kokoro()
        .with(&yaml("timeout_ms: 1000\nconcurrency: 16"))
        .unwrap();
    assert_eq!(a.version_string(), b.version_string());
}
