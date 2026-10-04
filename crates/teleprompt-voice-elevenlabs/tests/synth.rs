//! Speaking a line with ElevenLabs: the request its API gets, and the
//! audio and timings read back out of its answer.

mod stub;

use std::collections::BTreeMap;

use base64::Engine;
use stub::{spawn, Reply};
use teleprompt_plugin::voice::{SynthRequest, VoiceBackend};
use teleprompt_voice_elevenlabs::{ElevenLabsConfig, ElevenLabsVoice, DEFAULT_VOICE};

const VOICES: &str = "GET /v1/voices";

fn speech_route(voice: &'static str) -> &'static str {
    Box::leak(
        format!("POST /v1/text-to-speech/{voice}/with-timestamps?output_format=pcm_24000")
            .into_boxed_str(),
    )
}

/// A tenth of a second of raw 24 kHz PCM, and "Hi all" timed by character.
fn spoken() -> Reply {
    let pcm: Vec<u8> = std::iter::repeat_n([100u8, 0], 2400).flatten().collect();
    Reply::json(serde_json::json!({
        "audio_base64": base64::engine::general_purpose::STANDARD.encode(pcm),
        "alignment": {
            "characters": ["H", "i", " ", "a", "l", "l", "."],
            "character_start_times_seconds": [0.0, 0.05, 0.1, 0.12, 0.15, 0.18, 0.2],
            "character_end_times_seconds": [0.05, 0.1, 0.12, 0.15, 0.18, 0.2, 0.25],
        },
    }))
}

fn listed() -> Reply {
    Reply::json(serde_json::json!({ "voices": [
        { "voice_id": DEFAULT_VOICE, "name": "George" },
        { "voice_id": "abc123", "name": "Narrator" },
    ]}))
}

/// The key comes from a variable cargo sets for every test run.
fn backend(base_url: &str) -> ElevenLabsVoice {
    ElevenLabsVoice::new(ElevenLabsConfig {
        base_url: base_url.to_string(),
        api_key_env: "CARGO_PKG_NAME".to_string(),
        seed: Some(7),
        ..ElevenLabsConfig::default()
    })
    .unwrap()
}

fn req(voice: Option<&str>, speed: f64) -> SynthRequest {
    SynthRequest {
        text: "Hi all.".into(),
        locale: "en".into(),
        voice: voice.map(str::to_string),
        speed,
        instruct: None,
    }
}

#[tokio::test]
async fn a_line_is_spoken_with_its_words_timed() {
    let stub = spawn(BTreeMap::from([(speech_route(DEFAULT_VOICE), spoken())])).await;
    let said = backend(&stub.base_url)
        .synthesize(&req(None, 1.1))
        .await
        .unwrap();
    assert_eq!(said.pcm.sample_rate, 24_000);
    assert_eq!(said.pcm.duration_ms(), 100);
    let words = said.word_timings.unwrap();
    let got: Vec<_> = words
        .iter()
        .map(|w| (w.word.as_str(), w.start_ms, w.end_ms))
        .collect();
    assert_eq!(got, [("Hi", 0, 100), ("all.", 120, 250)]);

    let seen = stub.requests.lock().unwrap();
    let (_, head, body) = &seen[0];
    assert!(head
        .to_ascii_lowercase()
        .contains("xi-api-key: teleprompt-voice-elevenlabs"));
    let body: serde_json::Value = serde_json::from_slice(body).unwrap();
    assert_eq!(body["text"], "Hi all.");
    assert_eq!(body["model_id"], "eleven_multilingual_v2");
    assert_eq!(body["voice_settings"]["speed"], 1.1);
    assert_eq!(body["seed"], 7);
}

/// A voice is named as the account shows it, or by its id.
#[tokio::test]
async fn a_voice_is_found_by_its_name() {
    let stub = spawn(BTreeMap::from([
        (VOICES, listed()),
        (speech_route("abc123"), spoken()),
    ]))
    .await;
    let voice = backend(&stub.base_url);
    voice.synthesize(&req(Some("narrator"), 1.0)).await.unwrap();
    voice.synthesize(&req(Some("abc123"), 1.0)).await.unwrap();
    let err = voice
        .synthesize(&req(Some("Nobody"), 1.0))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("no voice `Nobody`"), "{err}");

    let names = voice.voices().await.unwrap().unwrap();
    assert!(names.contains(&"Narrator".to_string()) && names.contains(&"abc123".to_string()));
}

/// What ElevenLabs cannot do is said, not quietly dropped.
#[tokio::test]
async fn a_speed_or_instruction_it_cannot_take_is_refused() {
    let voice = backend("http://127.0.0.1:9");
    let err = voice
        .synthesize(&req(None, 2.0))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("0.7 to 1.2"), "{err}");
    let mut asked = req(None, 1.0);
    asked.instruct = Some("warmly".into());
    let err = voice.synthesize(&asked).await.unwrap_err().to_string();
    assert!(err.contains("voice.instruct"), "{err}");
}

#[tokio::test]
async fn a_refusal_says_why() {
    let stub = spawn(BTreeMap::from([(
        speech_route(DEFAULT_VOICE),
        Reply::status(
            401,
            serde_json::json!({ "detail": { "status": "invalid_api_key", "message": "Invalid API key" } }),
        ),
    )]))
    .await;
    let err = backend(&stub.base_url)
        .synthesize(&req(None, 1.0))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("401") && err.contains("Invalid API key"),
        "{err}"
    );
}

#[test]
fn settings_out_of_range_are_refused_and_the_rest_are_in_the_key() {
    let v = |s: &str| serde_yaml::from_str::<serde_yaml::Value>(s).unwrap();
    let err = ElevenLabsConfig::from_value(&v("stability: 2")).unwrap_err();
    assert!(err.contains("stability"), "{err}");
    let cfg = ElevenLabsConfig::from_value(&v("model: eleven_v3\nstyle: 0.5")).unwrap();
    assert_eq!(cfg.version_string(), "elevenlabs/eleven_v3/style0.5");
}
