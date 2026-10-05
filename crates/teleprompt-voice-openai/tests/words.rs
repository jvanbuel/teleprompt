//! Word timings, opt-in, from Kokoro-FastAPI's `/dev/captioned_speech`.

use teleprompt_testkit::http as stub;

use base64::Engine;
use stub::{spawn, Reply};
use teleprompt_voice::{SynthRequest, VoiceBackend};
use teleprompt_voice_openai::{OpenAiConfig, OpenAiVoice};

fn req() -> SynthRequest {
    SynthRequest {
        text: "Hello from teleprompt.".into(),
        locale: "en".into(),
        voice: Some("af_heart".into()),
        speed: 1.0,
        instruct: None,
    }
}

fn backend(base_url: &str, word_timings: bool) -> OpenAiVoice {
    OpenAiVoice::new(OpenAiConfig {
        base_url: base_url.to_string(),
        word_timings,
        ..OpenAiConfig::kokoro()
    })
    .unwrap()
}

/// The shape a current server answers with, verbatim apart from length.
fn captioned() -> Vec<u8> {
    let pcm: Vec<u8> = (0..2400i16).flat_map(i16::to_le_bytes).collect();
    serde_json::json!({
        "audio": base64::engine::general_purpose::STANDARD.encode(pcm),
        "audio_format": "pcm",
        "timestamps": [
            {"word": "Hello", "start_time": 0.0228, "end_time": 0.3478},
            {"word": "from", "start_time": 0.3478, "end_time": 0.4978},
            {"word": "teleprompt", "start_time": 0.4978, "end_time": 1.3228},
            {"word": ".", "start_time": 1.3228, "end_time": 1.4603}
        ]
    })
    .to_string()
    .into_bytes()
}

#[tokio::test]
async fn opted_in_the_words_come_with_the_audio_without_punctuation() {
    let s = spawn(Reply::ok(captioned())).await;
    let b = backend(&s.base_url, true);
    let out = b.synthesize(&req()).await.unwrap();
    assert_eq!(out.pcm.duration_ms(), 100);
    let words = out.word_timings.expect("timed");
    let said: Vec<(&str, u64)> = words
        .iter()
        .map(|w| (w.word.as_str(), w.start_ms))
        .collect();
    assert_eq!(said, [("Hello", 23), ("from", 348), ("teleprompt", 498)]);
    assert!(s.first().head.starts_with("POST /dev/captioned_speech"));
}

/// Off by default: the stable endpoint, and no words.
#[tokio::test]
async fn by_default_the_stable_endpoint_is_used() {
    let pcm: Vec<u8> = (0..2400i16).flat_map(i16::to_le_bytes).collect();
    let s = spawn(Reply::ok(pcm)).await;
    let b = backend(&s.base_url, false);
    assert!(b.synthesize(&req()).await.unwrap().word_timings.is_none());
    assert!(s.first().head.starts_with("POST /v1/audio/speech"));
}

/// A server without the endpoint fails the dub and names the setting,
/// rather than quietly losing the timings asked for.
#[tokio::test]
async fn a_server_without_captions_says_which_setting_to_change() {
    let s = spawn(Reply::error(404, "Not Found")).await;
    let err = backend(&s.base_url, true)
        .synthesize(&req())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("word_timings = false"), "{err}");
}

/// Turning timings on changes the cache key once, so entries synthesized
/// without them are not served as if they had them.
#[test]
fn timed_audio_is_cached_apart() {
    let off = OpenAiConfig::kokoro();
    let on = OpenAiConfig {
        word_timings: true,
        ..OpenAiConfig::kokoro()
    };
    assert_ne!(off.version_string(), on.version_string());
}

/// A word the server did not time is left out, not placed at 0 ms: a cue
/// on it then falls back to where it sits in the sentence, instead of
/// firing as the sentence starts.
#[tokio::test]
async fn a_word_without_times_is_left_out_rather_than_put_at_the_start() {
    let pcm: Vec<u8> = (0..2400i16).flat_map(i16::to_le_bytes).collect();
    let body = serde_json::json!({
        "audio": base64::engine::general_purpose::STANDARD.encode(pcm),
        "timestamps": [
            {"word": "Hello", "start_time": 0.0228, "end_time": 0.3478},
            {"word": "from"},
            {"word": "teleprompt", "start_time": "soon", "end_time": 1.3228}
        ]
    });
    let s = spawn(Reply::ok(body.to_string().into_bytes())).await;
    let out = backend(&s.base_url, true).synthesize(&req()).await.unwrap();
    let words = out.word_timings.expect("timed");
    let said: Vec<&str> = words.iter().map(|w| w.word.as_str()).collect();
    assert_eq!(said, ["Hello"]);
}
