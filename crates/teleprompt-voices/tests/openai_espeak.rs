//! The eSpeak server in `examples/voices`, a speech server written without
//! teleprompt in Python's standard library, spoken to as any server of the
//! API is. Skipped without python3 and espeak-ng.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use teleprompt_voice::SynthRequest;

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn runs(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// The server on a free port, once it answers.
async fn espeak_server() -> Option<(Server, u16)> {
    if !runs("python3") || !runs("espeak-ng") {
        eprintln!("skipped: no python3 or espeak-ng");
        return None;
    }
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/voices/espeak_server.py");
    let child = Command::new("python3")
        .arg(script)
        .arg(port.to_string())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let server = Server(child);
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Some((server, port));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the eSpeak server did not start on port {port}");
}

#[tokio::test]
async fn the_espeak_server_is_a_voice_with_nothing_but_settings() {
    let Some((_server, port)) = espeak_server().await else {
        return;
    };
    let settings: serde_yaml::Value = serde_yaml::from_str(&format!(
        "base_url: http://127.0.0.1:{port}/v1\nmodel: espeak"
    ))
    .unwrap();
    let voice = teleprompt_voices::openai::endpoint("espeak", &settings).unwrap();
    assert!(voice
        .voices()
        .await
        .unwrap()
        .unwrap()
        .contains(&"en-us".to_string()));

    let line = |speed: f64| SynthRequest {
        text: "Hello there, plugin authors.".into(),
        locale: "en".into(),
        voice: Some("en-us".into()),
        speed,
        instruct: None,
    };
    let said = voice.synthesize(&line(1.0)).await.unwrap();
    assert_eq!(said.pcm.sample_rate, 24_000);
    let ms = said.pcm.duration_ms();
    assert!((800..4000).contains(&ms), "{ms}ms");
    // Faster is shorter.
    let quick = voice.synthesize(&line(2.0)).await.unwrap();
    assert!(quick.pcm.duration_ms() < ms * 3 / 4, "{ms}ms");
}
