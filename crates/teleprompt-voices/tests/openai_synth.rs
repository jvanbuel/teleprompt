use teleprompt_testkit::http as stub;

use stub::{spawn, Reply};
use teleprompt_voice::{SynthRequest, VoiceBackend, VoiceError};
use teleprompt_voices::openai::{OpenAiConfig, OpenAiVoice, PCM_SAMPLE_RATE};

fn req(text: &str) -> SynthRequest {
    SynthRequest {
        text: text.to_string(),
        locale: "en".to_string(),
        voice: Some("af_heart".to_string()),
        speed: 1.0,
        instruct: None,
    }
}

fn backend(base_url: &str, timeout_ms: u64) -> OpenAiVoice {
    OpenAiVoice::new(OpenAiConfig {
        base_url: base_url.to_string(),
        timeout_ms,
        ..OpenAiConfig::kokoro()
    })
    .unwrap()
}

/// 2400 little-endian i16 samples = 4800 bytes = exactly 100 ms at 24 kHz.
fn pcm_bytes(samples: usize) -> Vec<u8> {
    (0..samples)
        .flat_map(|i| ((i % 1000) as i16).to_le_bytes())
        .collect()
}

#[tokio::test]
async fn a_successful_synthesis_decodes_to_24khz_mono() {
    let s = spawn(Reply::ok(pcm_bytes(2400))).await;
    let out = backend(&s.base_url, 30_000)
        .synthesize(&req("hello"))
        .await
        .unwrap();

    assert_eq!(out.pcm.sample_rate, PCM_SAMPLE_RATE);
    assert_eq!(out.pcm.channels, 1);
    assert_eq!(out.pcm.samples.len(), 2400);
    assert_eq!(out.pcm.duration_ms(), 100);
    // Word timings are opt-in, because Kokoro serves them only from a /dev/
    // path; off by default, none come back.
    assert!(out.word_timings.is_none());
}

#[tokio::test]
async fn the_request_body_is_what_the_spec_says() {
    let s = spawn(Reply::ok(pcm_bytes(2))).await;
    let r = SynthRequest {
        text: "hello".to_string(),
        locale: "en".to_string(),
        voice: Some("af_bella".to_string()),
        speed: 1.25,
        instruct: None,
    };
    backend(&s.base_url, 30_000).synthesize(&r).await.unwrap();

    assert_eq!(
        s.first().head.lines().next().unwrap().to_string(),
        "POST /v1/audio/speech HTTP/1.1"
    );
    let body = s.first().json();
    assert_eq!(body["model"], "kokoro");
    assert_eq!(body["input"], "hello");
    assert_eq!(body["voice"], "af_bella");
    assert_eq!(body["response_format"], "pcm");
    // Sent to the server so the voice is *generated* at this rate rather than
    // resampled (docs/design.md#principles).
    assert_eq!(body["speed"], 1.25);
}

#[tokio::test]
async fn an_odd_byte_count_is_an_error_not_a_dropped_sample() {
    // i16 samples are byte pairs. A trailing odd byte means the body is not
    // what it claims; silently dropping it would publish a duration one
    // sample short of the audio and hide a real protocol problem.
    let s = spawn(Reply::ok(vec![1, 2, 3])).await;
    let err = backend(&s.base_url, 30_000)
        .synthesize(&req("x"))
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("odd") || msg.contains("3 bytes"), "{msg}");
}

#[tokio::test]
async fn an_empty_body_is_an_error() {
    // Zero samples would be a zero-length WAV published as a real line.
    let s = spawn(Reply::ok(Vec::new())).await;
    let err = backend(&s.base_url, 30_000)
        .synthesize(&req("x"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("empty"), "{err}");
}

#[tokio::test]
async fn a_non_200_names_the_url_and_the_status() {
    let s = spawn(Reply::error(500, "boom")).await;
    let err = backend(&s.base_url, 30_000)
        .synthesize(&req("x"))
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("500"), "{msg}");
    assert!(msg.contains(&s.base_url), "{msg}");
    // Never a silent fallback to silence.
    assert!(!matches!(err, VoiceError::Unsupported { .. }));
}

#[tokio::test]
async fn a_timeout_names_the_url_and_the_limit() {
    let s = spawn(Reply::Hang).await;
    let err = backend(&s.base_url, 150)
        .synthesize(&req("x"))
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("150"), "{msg}");
    assert!(msg.contains(&s.base_url), "{msg}");
}

#[tokio::test]
async fn a_truncated_body_is_an_error_not_short_audio() {
    let s = spawn(Reply::Truncated {
        body: pcm_bytes(10),
        claim: 4800,
    })
    .await;
    let err = backend(&s.base_url, 30_000)
        .synthesize(&req("x"))
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(!msg.is_empty());
    assert!(msg.contains(&s.base_url), "{msg}");
    // Pins the actual path taken: `client.rs`'s own literal
    // "response body incomplete: {e}", not reqwest's internal wording.
    // Without this, the test would pass just as happily if a bug routed
    // this scenario through the connect-error or non-200 path instead.
    assert!(msg.contains("incomplete"), "{msg}");
}

#[tokio::test]
async fn an_unreachable_server_names_the_url() {
    // Port 1 on loopback: nothing listens, connection refused immediately.
    let err = backend("http://127.0.0.1:1", 30_000)
        .synthesize(&req("x"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("127.0.0.1:1"), "{err}");
    // Why, from beneath reqwest's own "error sending request".
    assert!(err.to_string().to_lowercase().contains("refused"), "{err}");
}
