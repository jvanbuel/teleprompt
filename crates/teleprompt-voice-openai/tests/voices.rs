mod stub;

use stub::{spawn, Reply};
use teleprompt_plugin::voice::VoiceBackend;
use teleprompt_voice_openai::{OpenAiConfig, OpenAiVoice};

fn backend(base_url: &str) -> OpenAiVoice {
    backend_with_timeout(base_url, OpenAiConfig::kokoro().timeout_ms)
}

fn backend_with_timeout(base_url: &str, timeout_ms: u64) -> OpenAiVoice {
    OpenAiVoice::new(OpenAiConfig {
        base_url: base_url.to_string(),
        timeout_ms,
        ..OpenAiConfig::kokoro()
    })
    .unwrap()
}

#[tokio::test]
async fn the_wrapper_object_shape_parses() {
    let s = spawn(Reply::Ok(
        br#"{"voices": ["af_heart", "af_bella"]}"#.to_vec(),
    ))
    .await;
    let voices = backend(&s.base_url)
        .voices()
        .await
        .expect("lists voices")
        .unwrap();
    assert_eq!(voices, vec!["af_heart".to_string(), "af_bella".to_string()]);
}

#[tokio::test]
async fn a_bare_top_level_array_parses_identically() {
    // The tolerance `client.rs` deliberately has for OpenAI-compatible
    // servers that skip the `{"voices": ...}` wrapper. Untested, this
    // tolerance is unproven.
    let s = spawn(Reply::Ok(br#"["af_heart", "af_bella"]"#.to_vec())).await;
    let voices = backend(&s.base_url)
        .voices()
        .await
        .expect("lists voices")
        .unwrap();
    assert_eq!(voices, vec!["af_heart".to_string(), "af_bella".to_string()]);
}

#[tokio::test]
async fn an_empty_voices_list_is_ok_not_an_error() {
    // "The server has no voices" is a real state a caller (`setup`, or
    // `dub`'s one-shot validation) should see and report, not an
    // exception raised on their behalf.
    let s = spawn(Reply::Ok(br#"{"voices": []}"#.to_vec())).await;
    let voices = backend(&s.base_url)
        .voices()
        .await
        .expect("lists voices")
        .unwrap();
    assert_eq!(voices, Vec::<String>::new());
}

#[tokio::test]
async fn json_without_a_voices_array_is_an_error() {
    // Valid JSON, but neither a `{"voices": [...]}` object nor a bare
    // array — the shape this backend understands is simply absent.
    let s = spawn(Reply::Ok(br#"{"error": "not supported"}"#.to_vec())).await;
    let err = backend(&s.base_url)
        .voices()
        .await
        .expect("lists voices")
        .unwrap_err();
    assert!(err.to_string().contains("no `voices` array"), "{err}");
}

#[tokio::test]
async fn a_non_200_names_the_status_and_the_url() {
    let s = spawn(Reply::Status(503, "down for maintenance".to_string())).await;
    let err = backend(&s.base_url)
        .voices()
        .await
        .expect("lists voices")
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("503"), "{msg}");
    assert!(msg.contains(&s.base_url), "{msg}");
}

#[tokio::test]
async fn a_body_that_is_not_json_at_all_is_a_decode_error() {
    let s = spawn(Reply::Ok(b"not json at all".to_vec())).await;
    let err = backend(&s.base_url)
        .voices()
        .await
        .expect("lists voices")
        .unwrap_err();
    // Pins `client.rs`'s own literal ("voice list was not JSON: {e}"), not
    // serde_json's internal wording.
    assert!(err.to_string().contains("was not JSON"), "{err}");
}

#[tokio::test]
async fn an_unreachable_server_names_the_url() {
    // Port 1 on loopback: nothing listens, connection refused immediately.
    let err = backend("http://127.0.0.1:1")
        .voices()
        .await
        .expect("lists voices")
        .unwrap_err();
    assert!(err.to_string().contains("127.0.0.1:1"), "{err}");
}

/// `speech()` already distinguishes a timeout from every other transport
/// failure (`a_timeout_names_the_url_and_the_limit` in `synth.rs`); `voices()`
/// used to map every `send()` error, timeout included, to the same "cannot list
/// voices: {e}" — and reqwest's `Display` drops the source chain, so a hang
/// read identically to a refused connection. `setup`'s probe is the caller
/// that most needs the distinction, since it is the command reached for when
/// something is broken.
#[tokio::test]
async fn a_timeout_names_the_limit_not_a_generic_transport_error() {
    let s = spawn(Reply::Hang).await;
    let err = backend_with_timeout(&s.base_url, 150)
        .voices()
        .await
        .expect("lists voices")
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("150"), "{msg}");
    assert!(
        !msg.contains("cannot list voices"),
        "a timeout has its own message, distinct from the generic transport-error one: {msg}"
    );
}

#[tokio::test]
async fn an_entry_that_is_not_a_voice_is_rejected_not_silently_dropped() {
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
    let err = backend(&s.base_url)
        .voices()
        .await
        .expect("lists voices")
        .unwrap_err();
    assert!(err.to_string().contains("neither a name"), "{err}");
}

/// Kokoro-FastAPI's voice list became OpenAI-compatible: each entry is an
/// object whose `id` is what `/v1/audio/speech` takes as `voice`. This is
/// the shape a current server answers, verbatim apart from its length.
#[tokio::test]
async fn voices_listed_as_objects_are_read_by_their_id() {
    let s = spawn(Reply::Ok(
        br#"{"voices": [
            {"id": "af_alloy", "name": "af_alloy", "overall_grade": "C", "target_quality": "B", "training_duration": "MM minutes"},
            {"id": "af_heart", "name": "af_heart", "overall_grade": "A", "target_quality": "A", "training_duration": "HH hours"}
        ]}"#
        .to_vec(),
    ))
    .await;
    let voices = backend(&s.base_url)
        .voices()
        .await
        .expect("lists voices")
        .expect("voices");
    assert_eq!(voices, vec!["af_alloy".to_string(), "af_heart".to_string()]);
}

/// An object with nothing to name the voice by is not a voice.
#[tokio::test]
async fn an_object_without_an_id_is_rejected() {
    let s = spawn(Reply::Ok(br#"{"voices": [{"grade": "A"}]}"#.to_vec())).await;
    let err = backend(&s.base_url)
        .voices()
        .await
        .expect("lists voices")
        .unwrap_err();
    assert!(err.to_string().contains("neither a name"), "{err}");
}
