//! `dub` through Gemini TTS: a project that names it speaks in a Gemini
//! voice, directed by its delivery instructions. The API is a stub.

mod http_stub;

use std::collections::BTreeMap;
use std::process::Command;

use base64::Engine;

#[tokio::test(flavor = "multi_thread")]
async fn a_project_speaks_in_its_gemini_voice() {
    let wav = teleprompt_plugin::voice::wav::encode(&teleprompt_plugin::voice::Pcm {
        sample_rate: 24_000,
        channels: 1,
        samples: vec![300; 24_000],
    });
    let answer = serde_json::json!({
        "steps": [{ "type": "model_output", "content": [{
            "type": "audio", "mime_type": "audio/wav",
            "data": base64::engine::general_purpose::STANDARD.encode(wav),
        }]}],
    });
    let (url, seen) = http_stub::serve(BTreeMap::from([(
        "POST /v1beta/interactions".to_string(),
        (answer.to_string().into_bytes(), "application/json"),
    )]))
    .await;
    let dir = teleprompt_testkit::test_dir("gemini-dub");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(
        dir.join("teleprompt.toml"),
        format!(
            "[voice]\nbackend = \"gemini\"\nvoice = \"Charon\"\ninstruct = \"calmly\"\n\n\
             [backends.gemini]\nbase_url = \"{url}\"\napi_key_env = \"CARGO_PKG_NAME\"\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("scripts/demo.md"),
        "---\nteleprompt: 1\n---\n\n# A\n\nWelcome to Acme. {#welcome}\n",
    )
    .unwrap();
    let out = tokio::task::spawn_blocking({
        let dir = dir.path().to_path_buf();
        move || {
            Command::new(env!("CARGO_BIN_EXE_teleprompt"))
                .current_dir(dir)
                .args(["dub", "scripts/demo.md", "--out", "out"])
                .output()
                .unwrap()
        }
    })
    .await
    .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("out/en/narration.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["lines"][0]["duration_ms"], 1000);
    let requests = seen.lock().unwrap().clone();
    let (_, body) = requests
        .iter()
        .find(|(r, _)| r == "POST /v1beta/interactions")
        .expect("spoke");
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["input"][0]["text"], "Welcome to Acme.");
    assert_eq!(body["input"][0]["annotations"][0]["style"], "calmly");
    assert_eq!(
        body["generation_config"]["speech_config"][0]["voice"],
        "Charon"
    );
}
