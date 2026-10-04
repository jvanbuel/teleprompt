//! Speaking a line through a Voicebox server: the profile named, the
//! request it gets, and what comes back.

mod stub;

use std::collections::BTreeMap;

use stub::{spawn, Reply};
use teleprompt_plugin::voice::{Pcm, SynthRequest, VoiceBackend};
use teleprompt_voice_voicebox::{VoiceboxConfig, VoiceboxVoice};

fn wav() -> Vec<u8> {
    teleprompt_plugin::voice::wav::encode(&Pcm {
        sample_rate: 24_000,
        channels: 1,
        samples: vec![100; 2400],
    })
}

fn profiles() -> Reply {
    Reply::json(serde_json::json!([
        { "id": "0b6f-jan", "name": "Jan" },
        { "id": "77aa-narrator", "name": "Narrator" }
    ]))
}

async fn server() -> stub::Stub {
    spawn(BTreeMap::from([
        ("GET /profiles", profiles()),
        ("POST /generate/stream", Reply::wav(wav())),
    ]))
    .await
}

fn backend(base_url: &str) -> VoiceboxVoice {
    VoiceboxVoice::new(VoiceboxConfig {
        base_url: base_url.to_string(),
        ..VoiceboxConfig::default()
    })
    .unwrap()
}

fn req(voice: Option<&str>) -> SynthRequest {
    SynthRequest {
        text: "Welcome to Acme.".into(),
        locale: "en-GB".into(),
        voice: voice.map(str::to_string),
        speed: 1.0,
        instruct: None,
    }
}

#[tokio::test]
async fn a_line_is_spoken_in_the_profile_it_names() {
    let s = server().await;
    let out = backend(&s.base_url)
        .synthesize(&req(Some("jan")))
        .await
        .unwrap();
    assert_eq!(out.pcm.duration_ms(), 100);
    assert!(out.word_timings.is_none());
    let body = s.json_to("POST /generate/stream");
    assert_eq!(body["profile_id"], "0b6f-jan");
    assert_eq!(body["text"], "Welcome to Acme.");
    assert_eq!(body["language"], "en");
    assert_eq!(body["engine"], "qwen");
    assert_eq!(body["seed"], 0);
    // Never rewritten in character: the words are the script's.
    assert_eq!(body["personality"], false);
    assert!(body.get("instruct").is_none(), "{body}");
}

#[tokio::test]
async fn a_profile_is_named_by_its_id_too_and_instructions_are_sent() {
    let s = server().await;
    let mut r = req(Some("77aa-narrator"));
    r.instruct = Some("warmly, with a smile".into());
    backend(&s.base_url).synthesize(&r).await.unwrap();
    let body = s.json_to("POST /generate/stream");
    assert_eq!(body["profile_id"], "77aa-narrator");
    assert_eq!(body["instruct"], "warmly, with a smile");
}

#[tokio::test]
async fn an_unknown_profile_names_the_ones_there_are() {
    let s = server().await;
    let err = backend(&s.base_url)
        .synthesize(&req(Some("Bob")))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Bob") && err.contains("Jan") && err.contains("Narrator"),
        "{err}"
    );
}

#[tokio::test]
async fn a_line_with_no_voice_says_how_to_make_one() {
    let s = server().await;
    let err = backend(&s.base_url)
        .synthesize(&req(None))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("voice.voice") && err.contains("teleprompt voice clone"),
        "{err}"
    );
}

/// Voicebox has no speed; ignoring one would publish a line at a pace
/// nobody asked for.
#[tokio::test]
async fn a_speed_is_refused() {
    let s = server().await;
    let mut r = req(Some("Jan"));
    r.speed = 1.2;
    let err = backend(&s.base_url)
        .synthesize(&r)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("voice.speed"), "{err}");
}

#[tokio::test]
async fn a_failing_server_says_so() {
    let s = spawn(BTreeMap::from([
        ("GET /profiles", profiles()),
        (
            "POST /generate/stream",
            Reply(500, "text/plain", b"model not loaded".to_vec()),
        ),
    ]))
    .await;
    let err = backend(&s.base_url)
        .synthesize(&req(Some("Jan")))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("500") && err.contains("model not loaded"),
        "{err}"
    );
}

/// What changes the voice is in the cache key: the engine and the seed.
#[test]
fn the_engine_and_seed_are_the_backends_version() {
    let version = |cfg: VoiceboxConfig| VoiceboxVoice::new(cfg).unwrap().capabilities().version;
    let plain = version(VoiceboxConfig::default());
    let chatterbox = version(VoiceboxConfig {
        engine: "chatterbox".into(),
        ..VoiceboxConfig::default()
    });
    let seeded = version(VoiceboxConfig {
        seed: 7,
        ..VoiceboxConfig::default()
    });
    assert_ne!(plain, chatterbox);
    assert_ne!(plain, seeded);
    assert!(
        !VoiceboxVoice::new(VoiceboxConfig::default())
            .unwrap()
            .capabilities()
            .speed_control
    );
}
