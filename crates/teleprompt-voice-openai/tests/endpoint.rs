//! A server of the author's own, under a name of theirs, and OpenAI's.

mod stub;

use stub::{spawn, Reply};
use teleprompt_plugin::voice::SynthRequest;
use teleprompt_voice_openai::{endpoint, OpenAiConfig};

fn settings(toml: &str) -> serde_yaml::Value {
    serde_yaml::from_str(toml).unwrap()
}

fn line() -> SynthRequest {
    SynthRequest {
        text: "Hello.".to_string(),
        locale: "en".to_string(),
        voice: None,
        speed: 1.0,
        instruct: Some("warmly".to_string()),
    }
}

#[test]
fn an_endpoint_needs_an_address() {
    let err = endpoint("studio", &settings("api: openai")).err().unwrap();
    assert!(
        err.contains("backends.studio") && err.contains("base_url"),
        "{err}"
    );
}

#[test]
fn an_endpoint_speaks_only_the_openai_api() {
    let err = endpoint("studio", &settings("api: elevenlabs\nbase_url: http://x"))
        .err()
        .unwrap();
    assert!(err.contains("\"openai\""), "{err}");
}

#[test]
fn a_bare_server_address_gets_the_api_path() {
    let at = |url: &str| {
        OpenAiConfig::endpoint("studio")
            .with(&settings(&format!("base_url: {url}")))
            .unwrap()
    };
    assert_eq!(at("http://box:8880").api_root(), "http://box:8880/v1");
    assert_eq!(at("http://box:8880/").api_root(), "http://box:8880/v1");
    assert_eq!(at("http://box/tts/v1").api_root(), "http://box/tts/v1");
    assert_eq!(at("http://box:8880/v1").server_root(), "http://box:8880");
}

#[test]
fn the_cache_key_names_the_model_and_an_unusual_rate() {
    let c = OpenAiConfig::endpoint("studio")
        .with(&settings(
            "base_url: http://x\nmodel: piper\nsample_rate: 22050",
        ))
        .unwrap();
    assert_eq!(c.version_string(), "piper@22050");
    assert_eq!(OpenAiConfig::kokoro().version_string(), "kokoro");
}

#[tokio::test]
async fn an_endpoint_is_named_by_the_author_and_says_lines_at_its_rate() {
    let s = spawn(Reply::Ok(vec![0; 2 * 22050])).await;
    let voice = endpoint(
        "studio",
        &settings(&format!(
            "api: openai\nbase_url: {}/v1\nvoice: amy\nsample_rate: 22050",
            s.base_url
        )),
    )
    .unwrap();
    assert_eq!(voice.id(), "studio");
    let out = voice.synthesize(&line()).await.unwrap();
    assert_eq!(out.pcm.duration_ms(), 1000);
    assert_eq!(s.first_request_line(), "POST /v1/audio/speech HTTP/1.1");
    let body = s.first_body();
    assert_eq!(body["voice"], "amy");
    assert_eq!(body["instructions"], "warmly");
}

#[tokio::test]
async fn an_endpoint_that_lists_no_voices_is_not_an_error() {
    let s = spawn(Reply::Status(404, "Not Found".into())).await;
    let voice = endpoint("studio", &settings(&format!("base_url: {}", s.base_url))).unwrap();
    assert!(voice.voices().await.is_none());
}

#[tokio::test]
async fn the_key_comes_from_the_variable_the_settings_name() {
    let s = spawn(Reply::Ok(vec![0; 480])).await;
    let var = "TELEPROMPT_TEST_SPEECH_KEY";
    let voice = endpoint(
        "studio",
        &settings(&format!("base_url: {}\napi_key_env: {var}", s.base_url)),
    )
    .unwrap();

    // Unset, it is named, and nothing is sent.
    std::env::remove_var(var);
    let err = voice.synthesize(&line()).await.unwrap_err().to_string();
    assert!(err.contains(var), "{err}");
    assert!(s.requests.lock().unwrap().is_empty());

    std::env::set_var(var, "sk-test");
    voice.synthesize(&line()).await.unwrap();
    let raw = s.requests.lock().unwrap()[0].to_lowercase();
    assert!(raw.contains("authorization: bearer sk-test"), "{raw}");
}

#[tokio::test]
async fn openai_lists_no_voices_and_asks_for_none() {
    let voice = (teleprompt_voice_openai::openai().build)(None).unwrap();
    assert_eq!(voice.id(), "openai");
    assert!(voice.voices().await.is_none());
    assert!(voice.capabilities().version.starts_with("gpt-4o-mini-tts"));
}
