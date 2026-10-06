//! Speaking a line with a Gemini TTS model: the request the Interactions
//! API gets, and the audio read back out of its answer.

use teleprompt_testkit::http as stub;

use std::collections::BTreeMap;

use base64::Engine;
use stub::{spawn, Reply};
use teleprompt_voice::{Pcm, SynthRequest, VoiceBackend};
use teleprompt_voices::gemini::{GeminiConfig, GeminiVoice};

const ROUTE: &str = "POST /v1beta/interactions";

/// A tenth of a second at 24 kHz, as a WAV: what a unary request returns.
fn wav() -> Vec<u8> {
    teleprompt_voice::wav::encode(&Pcm {
        sample_rate: 24_000,
        channels: 1,
        samples: vec![100; 2400],
    })
}

/// An interaction whose last model output holds `audio`.
fn answer(audio: serde_json::Value) -> Reply {
    Reply::json(serde_json::json!({
        "id": "int_1",
        "status": "completed",
        "steps": [
            { "type": "user_input", "content": [{ "type": "text", "text": "…" }] },
            { "type": "model_output", "content": [audio] },
        ],
    }))
}

fn wav_answer() -> Reply {
    let data = base64::engine::general_purpose::STANDARD.encode(wav());
    answer(serde_json::json!({
        "type": "audio", "mime_type": "audio/wav", "data": data,
        "sample_rate": 24000, "channels": 1,
    }))
}

/// The key comes from a variable cargo sets for every test run, so no
/// test has to set one while others read the environment.
fn backend(base_url: &str) -> GeminiVoice {
    GeminiVoice::new(GeminiConfig {
        base_url: base_url.to_string(),
        api_key_env: "CARGO_PKG_NAME".to_string(),
        ..GeminiConfig::default()
    })
    .unwrap()
}

fn req(voice: Option<&str>, instruct: Option<&str>) -> SynthRequest {
    SynthRequest {
        text: "Welcome to Acme.".into(),
        locale: "en".into(),
        voice: voice.map(str::to_string),
        speed: 1.0,
        instruct: instruct.map(str::to_string),
    }
}

#[tokio::test]
async fn a_line_is_spoken_in_the_voice_it_names() {
    let s = spawn(BTreeMap::from([(ROUTE, wav_answer())])).await;
    let out = backend(&s.base_url)
        .synthesize(&req(Some("Puck"), None))
        .await
        .unwrap();
    assert_eq!(out.pcm.duration_ms(), 100);
    assert_eq!(out.pcm.sample_rate, 24_000);

    let body = s.json_to(ROUTE);
    assert_eq!(body["model"], "gemini-3.8-flash-tts");
    assert_eq!(body["input"][0]["type"], "text");
    assert_eq!(body["input"][0]["text"], "Welcome to Acme.");
    assert!(body["input"][0].get("annotations").is_none(), "{body}");
    assert_eq!(body["response_format"]["type"], "audio");
    assert_eq!(
        body["generation_config"]["speech_config"],
        serde_json::json!([{ "voice": "Puck" }])
    );
    assert!(
        s.head_of(ROUTE)
            .contains("x-goog-api-key: teleprompt-voices"),
        "{}",
        s.head_of(ROUTE)
    );
}

/// The words are spoken verbatim; how to say them goes beside them, where
/// the model reads it as direction rather than as more to say.
#[tokio::test]
async fn delivery_is_direction_not_words() {
    let s = spawn(BTreeMap::from([(ROUTE, wav_answer())])).await;
    backend(&s.base_url)
        .synthesize(&req(None, Some("calmly, a little slower")))
        .await
        .unwrap();
    let body = s.json_to(ROUTE);
    assert_eq!(body["input"][0]["text"], "Welcome to Acme.");
    assert_eq!(
        body["input"][0]["annotations"],
        serde_json::json!([{ "type": "speech_metadata", "style": "calmly, a little slower" }])
    );
    // No voice named: the default one.
    assert_eq!(
        body["generation_config"]["speech_config"][0]["voice"],
        "Kore"
    );
}

/// A streamed or older answer is raw 16-bit PCM at the rate it states.
#[tokio::test]
async fn raw_pcm_is_read_at_its_rate() {
    let pcm: Vec<u8> = std::iter::repeat_n([0x10u8, 0x00], 1600)
        .flatten()
        .collect();
    let data = base64::engine::general_purpose::STANDARD.encode(pcm);
    let s = spawn(BTreeMap::from([(
        ROUTE,
        answer(serde_json::json!({
            "type": "audio", "mime_type": "audio/l16", "data": data, "sample_rate": 16000,
        })),
    )]))
    .await;
    let out = backend(&s.base_url)
        .synthesize(&req(None, None))
        .await
        .unwrap();
    assert_eq!(out.pcm.sample_rate, 16_000);
    assert_eq!(out.pcm.duration_ms(), 100);
}

/// A MIME type is case-insensitive, and Gemini's raw audio usually states
/// its rate in the type itself: `audio/L16;codec=pcm;rate=24000`.
#[tokio::test]
async fn raw_pcm_is_read_at_the_rate_its_type_states() {
    let pcm: Vec<u8> = std::iter::repeat_n([0x10u8, 0x00], 1600)
        .flatten()
        .collect();
    let data = base64::engine::general_purpose::STANDARD.encode(pcm);
    let s = spawn(BTreeMap::from([(
        ROUTE,
        answer(serde_json::json!({
            "type": "audio", "mime_type": "audio/L16;codec=pcm;rate=16000", "data": data,
        })),
    )]))
    .await;
    let out = backend(&s.base_url)
        .synthesize(&req(None, None))
        .await
        .unwrap();
    assert_eq!(out.pcm.sample_rate, 16_000);
    assert_eq!(out.pcm.duration_ms(), 100);
}

#[tokio::test]
async fn the_model_and_seed_are_the_projects() {
    let s = spawn(BTreeMap::from([(ROUTE, wav_answer())])).await;
    let voice = GeminiVoice::new(GeminiConfig {
        base_url: s.base_url.clone(),
        api_key_env: "CARGO_PKG_NAME".to_string(),
        model: "gemini-3.8-flash-lite-tts".to_string(),
        seed: Some(7),
        ..GeminiConfig::default()
    })
    .unwrap();
    voice.synthesize(&req(None, None)).await.unwrap();
    let body = s.json_to(ROUTE);
    assert_eq!(body["model"], "gemini-3.8-flash-lite-tts");
    assert_eq!(body["generation_config"]["seed"], 7);
    // Each changes the audio, so each is in the cache key.
    let version = voice.version();
    assert!(
        version.contains("gemini-3.8-flash-lite-tts") && version.contains("seed7"),
        "{version}"
    );
}

#[tokio::test]
async fn no_key_says_where_to_get_one() {
    let voice = GeminiVoice::new(GeminiConfig {
        api_key_env: "TELEPROMPT_TEST_NO_SUCH_KEY".to_string(),
        ..GeminiConfig::default()
    })
    .unwrap();
    let err = voice
        .synthesize(&req(None, None))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("TELEPROMPT_TEST_NO_SUCH_KEY") && err.contains("aistudio.google.com"),
        "{err}"
    );
}

/// An error from the API is its own message, not just a status.
#[tokio::test]
async fn a_refusal_says_why() {
    let s = spawn(BTreeMap::from([(
        ROUTE,
        Reply::error(
            429,
            serde_json::json!({ "error": { "code": 429, "message": "Resource has been exhausted (e.g. check quota).", "status": "RESOURCE_EXHAUSTED" } }),
        ),
    )]))
    .await;
    let err = backend(&s.base_url)
        .synthesize(&req(None, None))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("429") && err.contains("exhausted") && err.contains("concurrency"),
        "{err}"
    );
}

#[tokio::test]
async fn an_answer_without_audio_is_an_error() {
    let s = spawn(BTreeMap::from([(
        ROUTE,
        answer(serde_json::json!({ "type": "text", "text": "I can't say that." })),
    )]))
    .await;
    let err = backend(&s.base_url)
        .synthesize(&req(None, None))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("no audio") && err.contains("say that"),
        "{err}"
    );
}

/// There is no speed to send: a pace is asked for in words.
#[tokio::test]
async fn a_speed_is_refused_with_the_way_to_ask_for_one() {
    let mut r = req(None, None);
    r.speed = 1.2;
    let err = backend("http://127.0.0.1:1")
        .synthesize(&r)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("voice.instruct"), "{err}");
}

#[test]
fn settings_are_checked() {
    let parse = |yaml: &str| GeminiConfig::from_value(&serde_yaml::from_str(yaml).unwrap());
    assert!(parse("model: gemini-3.8-flash-lite-tts\nconcurrency: 2").is_ok());
    assert!(parse("concurrency: 0").unwrap_err().contains("concurrency"));
    assert!(parse("voice_id: Kore").unwrap_err().contains("voice_id"));
    assert!(parse("model: ''").unwrap_err().contains("model"));
}
