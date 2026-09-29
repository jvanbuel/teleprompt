//! `dub` through a Voicebox server: a project that names it speaks in a
//! Voicebox voice, with its delivery instructions. The server is a stub.

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

type Seen = Arc<Mutex<Vec<(String, String)>>>;

/// Answers the profile list and speech routes; records each request's route
/// and body.
async fn stub(wav: Vec<u8>) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let routes = Arc::new(BTreeMap::from([
        (
            "GET /profiles".to_string(),
            (
                r#"[{"id":"p-1","name":"Jan"}]"#.as_bytes().to_vec(),
                "application/json",
            ),
        ),
        ("POST /generate/stream".to_string(), (wav, "audio/wav")),
        (
            "POST /profiles".to_string(),
            (
                r#"{"id":"p-2","name":"Narrator"}"#.as_bytes().to_vec(),
                "application/json",
            ),
        ),
        (
            "POST /profiles/p-2/samples".to_string(),
            (r#"{"id":"s-1"}"#.as_bytes().to_vec(), "application/json"),
        ),
    ]));
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let (log, routes) = (log.clone(), routes.clone());
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 65536];
                loop {
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    raw.extend_from_slice(&buf[..n]);
                    // Counted in bytes: a WAV body is not text.
                    let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
                        if n == 0 {
                            return;
                        }
                        continue;
                    };
                    let head = String::from_utf8_lossy(&raw[..end]).to_string();
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if raw.len() < end + 4 + length && n > 0 {
                        continue;
                    }
                    let body = String::from_utf8_lossy(&raw[end + 4..]).to_string();
                    let route: String = head.split(' ').take(2).collect::<Vec<_>>().join(" ");
                    log.lock().unwrap().push((route.clone(), body));
                    let (reply, kind) = routes
                        .get(&route)
                        .cloned()
                        .unwrap_or((b"no".to_vec(), "text/plain"));
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.len());
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&reply).await;
                    return;
                }
            });
        }
    });
    (url, seen)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_speaks_in_its_voicebox_voice() {
    let wav = teleprompt_voice::wav::encode(&teleprompt_voice::Pcm {
        sample_rate: 24_000,
        channels: 1,
        samples: vec![300; 24_000],
    });
    let (url, seen) = stub(wav).await;
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
    let requests = seen.lock().unwrap().clone();
    let (_, body) = requests
        .iter()
        .find(|(r, _)| r == "POST /generate/stream")
        .expect("spoke");
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
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
    let (url, seen) = stub(Vec::new()).await;
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
    let requests = seen.lock().unwrap().clone();
    let samples: Vec<&String> = requests
        .iter()
        .filter(|(r, _)| r == "POST /profiles/p-2/samples")
        .map(|(_, b)| b)
        .collect();
    assert_eq!(samples.len(), 2, "{requests:?}");
    // The longest first.
    assert!(samples[0].contains("Deployment is one command."));
    assert!(samples[1].contains("Welcome to Acme."));
}
