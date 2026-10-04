//! `setup`'s question: does the key open the model, without paying for
//! a line of speech.

mod stub;

use std::collections::BTreeMap;

use stub::{spawn, Reply};
use teleprompt_voice_gemini::{GeminiConfig, GeminiVoice};

const ROUTE: &str = "GET /v1beta/models/gemini-3.8-flash-tts";

fn backend(base_url: &str, key_env: &str) -> GeminiVoice {
    GeminiVoice::new(GeminiConfig {
        base_url: base_url.to_string(),
        api_key_env: key_env.to_string(),
        ..GeminiConfig::default()
    })
    .unwrap()
}

#[tokio::test]
async fn a_key_that_opens_the_model_says_so() {
    let stub = spawn(BTreeMap::from([(
        ROUTE,
        Reply::json(serde_json::json!({
            "name": "models/gemini-3.8-flash-tts",
            "displayName": "Gemini 3.8 Flash TTS",
        })),
    )]))
    .await;
    let said = backend(&stub.base_url, "CARGO_PKG_NAME")
        .check()
        .await
        .unwrap();
    assert!(said.contains("Gemini 3.8 Flash TTS"), "{said}");
    let seen = stub.requests.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0]
        .1
        .to_lowercase()
        .contains("x-goog-api-key: teleprompt-voice-gemini"));
}

#[tokio::test]
async fn a_refused_key_is_said_in_the_apis_words() {
    let stub = spawn(BTreeMap::from([(
        ROUTE,
        Reply::status(
            400,
            serde_json::json!({ "error": { "message": "API key not valid." } }),
        ),
    )]))
    .await;
    let e = backend(&stub.base_url, "CARGO_PKG_NAME")
        .check()
        .await
        .unwrap_err();
    assert!(e.to_string().contains("API key not valid"), "{e}");
}

#[tokio::test]
async fn no_key_is_said_before_anything_is_sent() {
    let stub = spawn(BTreeMap::new()).await;
    let e = backend(&stub.base_url, "TELEPROMPT_NO_SUCH_KEY_VAR")
        .check()
        .await
        .unwrap_err();
    assert!(e.to_string().contains("TELEPROMPT_NO_SUCH_KEY_VAR"), "{e}");
    assert!(stub.requests.lock().unwrap().is_empty());
}
