//! Making a Voicebox voice from recordings and what they say: a profile,
//! then each recording as a sample with its text.

mod stub;

use std::collections::BTreeMap;

use stub::{spawn, Reply};
use teleprompt_voice_voicebox::{Sample, VoiceboxConfig, VoiceboxVoice};

fn backend(base_url: &str) -> VoiceboxVoice {
    VoiceboxVoice::new(VoiceboxConfig {
        base_url: base_url.to_string(),
        ..VoiceboxConfig::default()
    })
    .unwrap()
}

fn sample(file: &str, text: &str) -> Sample {
    Sample {
        file: file.to_string(),
        wav: b"RIFF....WAVEfmt ".to_vec(),
        text: text.to_string(),
    }
}

#[tokio::test]
async fn a_voice_is_a_profile_and_its_samples_with_their_text() {
    let s = spawn(BTreeMap::from([
        ("GET /profiles", Reply::json(serde_json::json!([]))),
        (
            "POST /profiles",
            Reply::json(serde_json::json!({ "id": "p-9", "name": "Jan" })),
        ),
        (
            "POST /profiles/p-9/samples",
            Reply::json(serde_json::json!({ "id": "s-1" })),
        ),
    ]))
    .await;
    let profile = backend(&s.base_url)
        .clone_voice(
            "Jan",
            "en",
            &[
                sample("welcome.wav", "Welcome to Acme."),
                sample("deploy.wav", "Deployment is one command."),
            ],
        )
        .await
        .unwrap();
    assert_eq!(profile.id, "p-9");
    let created = s.json_to("POST /profiles");
    assert_eq!(created["name"], "Jan");
    assert_eq!(created["language"], "en");
    assert_eq!(created["voice_type"], "cloned");
    assert_eq!(created["default_engine"], "qwen");
    let samples = s.bodies_to("POST /profiles/p-9/samples");
    assert_eq!(samples.len(), 2);
    let first = String::from_utf8_lossy(&samples[0]);
    assert!(
        first.contains("name=\"reference_text\"") && first.contains("Welcome to Acme."),
        "{first}"
    );
    assert!(
        first.contains("filename=\"welcome.wav\"") && first.contains("RIFF"),
        "{first}"
    );
}

#[tokio::test]
async fn a_name_already_taken_is_refused_before_anything_is_made() {
    let s = spawn(BTreeMap::from([(
        "GET /profiles",
        Reply::json(serde_json::json!([{ "id": "p-1", "name": "Jan" }])),
    )]))
    .await;
    let err = backend(&s.base_url)
        .clone_voice("jan", "en", &[sample("a.wav", "A.")])
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("Jan") && err.contains("already"), "{err}");
    assert!(s.bodies_to("POST /profiles").is_empty());
}
