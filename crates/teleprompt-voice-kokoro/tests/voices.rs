mod stub;

use stub::{spawn, Reply};
use teleprompt_voice_kokoro::{KokoroConfig, KokoroVoice};

fn backend(base_url: &str) -> KokoroVoice {
    KokoroVoice::new(KokoroConfig {
        base_url: base_url.to_string(),
        ..KokoroConfig::default()
    })
    .unwrap()
}

#[tokio::test]
async fn the_wrapper_object_shape_parses() {
    let s = spawn(Reply::Ok(
        br#"{"voices": ["af_heart", "af_bella"]}"#.to_vec(),
    ))
    .await;
    let voices = backend(&s.base_url).voices().await.unwrap();
    assert_eq!(voices, vec!["af_heart".to_string(), "af_bella".to_string()]);
}

#[tokio::test]
async fn a_bare_top_level_array_parses_identically() {
    // The tolerance `client.rs` deliberately has for OpenAI-compatible
    // servers that skip the `{"voices": ...}` wrapper. Untested, this
    // tolerance is unproven.
    let s = spawn(Reply::Ok(br#"["af_heart", "af_bella"]"#.to_vec())).await;
    let voices = backend(&s.base_url).voices().await.unwrap();
    assert_eq!(voices, vec!["af_heart".to_string(), "af_bella".to_string()]);
}

#[tokio::test]
async fn an_empty_voices_list_is_ok_not_an_error() {
    // "The server has no voices" is a real state a caller (`doctor`, or
    // `dub`'s one-shot validation) should see and report, not an
    // exception raised on their behalf.
    let s = spawn(Reply::Ok(br#"{"voices": []}"#.to_vec())).await;
    let voices = backend(&s.base_url).voices().await.unwrap();
    assert_eq!(voices, Vec::<String>::new());
}

#[tokio::test]
async fn json_without_a_voices_array_is_an_error() {
    // Valid JSON, but neither a `{"voices": [...]}` object nor a bare
    // array — the shape this backend understands is simply absent.
    let s = spawn(Reply::Ok(br#"{"error": "not supported"}"#.to_vec())).await;
    let err = backend(&s.base_url).voices().await.unwrap_err();
    assert!(err.to_string().contains("no `voices` array"), "{err}");
}

#[tokio::test]
async fn a_non_200_names_the_status_and_the_url() {
    let s = spawn(Reply::Status(503, "down for maintenance".to_string())).await;
    let err = backend(&s.base_url).voices().await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("503"), "{msg}");
    assert!(msg.contains(&s.base_url), "{msg}");
}

#[tokio::test]
async fn a_body_that_is_not_json_at_all_is_a_decode_error() {
    let s = spawn(Reply::Ok(b"not json at all".to_vec())).await;
    let err = backend(&s.base_url).voices().await.unwrap_err();
    // Pins `client.rs`'s own literal ("voice list was not JSON: {e}"), not
    // serde_json's internal wording.
    assert!(err.to_string().contains("was not JSON"), "{err}");
}

#[tokio::test]
async fn an_unreachable_server_names_the_url() {
    // Port 1 on loopback: nothing listens, connection refused immediately.
    let err = backend("http://127.0.0.1:1").voices().await.unwrap_err();
    assert!(err.to_string().contains("127.0.0.1:1"), "{err}");
}

#[tokio::test]
async fn a_non_string_entry_is_rejected_not_silently_dropped() {
    // Decision: reject rather than filter_map it away. A stray number or
    // null mixed into the array is much more likely to mean the server's
    // response shape changed under us than that it is an intentional
    // non-string voice name, and silently dropping it would hand back a
    // shorter-but-plausible-looking list instead of surfacing that. Same
    // reasoning `decode_pcm` uses for rejecting an odd byte count instead
    // of dropping the trailing byte.
    let s = spawn(Reply::Ok(
        br#"{"voices": ["af_heart", 42, "af_bella"]}"#.to_vec(),
    ))
    .await;
    let err = backend(&s.base_url).voices().await.unwrap_err();
    assert!(err.to_string().contains("non-string"), "{err}");
}
