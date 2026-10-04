//! The one test that needs a real Kokoro-FastAPI server. Not run in CI.
//!
//! Start one, then: `cargo test -p teleprompt-voice-kokoro --test real -- --ignored`

use teleprompt_plugin::voice::{SynthRequest, VoiceBackend};
use teleprompt_voice_kokoro::{KokoroConfig, KokoroVoice};

#[tokio::test]
#[ignore = "needs a Kokoro-FastAPI server on localhost:8880"]
async fn a_real_server_produces_plausible_audio() {
    let k = KokoroVoice::new(KokoroConfig::default()).unwrap();

    let voices = k.voices().await.expect("server reachable");
    assert!(!voices.is_empty(), "server reported no voices");

    let out = k
        .synthesize(&SynthRequest {
            text: "The quick brown fox jumps over the lazy dog.".to_string(),
            locale: "en".to_string(),
            voice: Some(voices[0].clone()),
            speed: 1.0,
            instruct: None,
        })
        .await
        .expect("synthesis succeeded");

    assert_eq!(out.pcm.sample_rate, 24_000);
    assert_eq!(out.pcm.channels, 1);
    // A nine-word sentence should land somewhere between half a second and
    // fifteen. Wider than any plausible voice, narrow enough to catch a
    // decode that produced garbage.
    let ms = out.pcm.duration_ms();
    assert!((500..15_000).contains(&ms), "implausible duration {ms}ms");
}
