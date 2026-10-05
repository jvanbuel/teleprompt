//! `dub` through a Voicebox server: a project that names it speaks in a
//! Voicebox voice, with its delivery instructions. The server is a stub.

use std::collections::BTreeMap;
use std::process::Command;

use serde_json::json;
use teleprompt_testkit::http::{self, Reply, Stub};

/// Answers the profile list and speech routes; records each request.
async fn stub(wav: Vec<u8>) -> Stub {
    http::spawn(BTreeMap::from([
        (
            "GET /profiles",
            Reply::json(json!([{"id": "p-1", "name": "Jan"}])),
        ),
        ("POST /generate/stream", Reply::wav(wav)),
        (
            "POST /profiles",
            Reply::json(json!({"id": "p-2", "name": "Narrator"})),
        ),
        (
            "POST /profiles/p-2/samples",
            Reply::json(json!({"id": "s-1"})),
        ),
    ]))
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_speaks_in_its_voicebox_voice() {
    let wav = teleprompt_voice::wav::encode(&teleprompt_voice::Pcm {
        sample_rate: 24_000,
        channels: 1,
        samples: vec![300; 24_000],
    });
    let stub = stub(wav).await;
    let url = &stub.base_url;
    let dir = teleprompt_testkit::test_dir("voicebox-dub");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(
        dir.join("teleprompt.toml"),
        format!("[voice]\nbackend = \"voicebox\"\nvoice = \"Jan\"\ninstruct = \"calmly\"\n\n[backends.voicebox]\nbase_url = \"{url}\"\n"),
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
    let body = stub.json_to("POST /generate/stream");
    assert_eq!(body["profile_id"], "p-1");
    assert_eq!(body["instruct"], "calmly");
    assert_eq!(body["personality"], false);
}

fn tp(
    dir: &std::path::Path,
    args: &'static [&'static str],
) -> tokio::task::JoinHandle<std::process::Output> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_teleprompt"))
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap()
    })
}

/// Your takes become a voice: each sent with the words it says, and the
/// settings that speak in it printed.
#[tokio::test(flavor = "multi_thread")]
async fn voice_clone_makes_a_voice_from_your_takes() {
    let stub = stub(Vec::new()).await;
    let url = &stub.base_url;
    let dir = teleprompt_testkit::test_dir("voicebox-clone");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(
        dir.join("teleprompt.toml"),
        format!("[backends.voicebox]\nbase_url = \"{url}\"\n"),
    )
    .unwrap();

    let none = tp(&dir, &["voice", "clone", "Narrator"]).await.unwrap();
    assert!(!none.status.success());
    assert!(
        String::from_utf8_lossy(&none.stderr).contains("no takes"),
        "{}",
        String::from_utf8_lossy(&none.stderr)
    );

    let mut takes = teleprompt_voice::takes::Takes::load(&dir.join("takes")).unwrap();
    let pcm = |s: usize| teleprompt_voice::Pcm {
        sample_rate: 16_000,
        channels: 1,
        samples: vec![200; s],
    };
    takes
        .save("welcome", "Welcome to Acme.", &pcm(48_000))
        .unwrap();
    takes
        .save("deploy", "Deployment is one command.", &pcm(64_000))
        .unwrap();
    // Too short to learn a voice from.
    takes.save("ok", "OK.", &pcm(8_000)).unwrap();

    let out = tp(&dir, &["voice", "clone", "Narrator"]).await.unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("voice = \"Narrator\"") && stdout.contains("backend = \"voicebox\""),
        "{stdout}"
    );
    let samples: Vec<String> = stub
        .requests_to("POST /profiles/p-2/samples")
        .iter()
        .map(http::Request::text)
        .collect();
    assert_eq!(samples.len(), 2, "{:?}", stub.requests());
    // The longest first.
    assert!(samples[0].contains("Deployment is one command."));
    assert!(samples[1].contains("Welcome to Acme."));
}
