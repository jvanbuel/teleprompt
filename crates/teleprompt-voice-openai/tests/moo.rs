//! The moo server in `examples/moo`, a speech server written without
//! teleprompt, spoken to as any server of the API is.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use teleprompt_plugin::voice::SynthRequest;

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

/// The server on a free port, once it answers; `None` without Python.
async fn moo_server() -> Option<(Server, u16)> {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/moo/deck/snippets/moo_server.py");
    let Ok(child) = Command::new("python3")
        .arg(script)
        .arg(port.to_string())
        .stderr(Stdio::null())
        .spawn()
    else {
        eprintln!("skipped: no python3 to run the moo server");
        return None;
    };
    let server = Server(child);
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Some((server, port));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the moo server did not start on port {port}");
}

#[tokio::test]
async fn the_moo_server_is_a_voice_with_nothing_but_settings() {
    let Some((_server, port)) = moo_server().await else {
        return;
    };
    let settings: serde_yaml::Value = serde_yaml::from_str(&format!(
        "api: openai\nbase_url: http://127.0.0.1:{port}/v1\nmodel: moo-1"
    ))
    .unwrap();
    let voice = teleprompt_voice_openai::endpoint("moo", &settings).unwrap();

    let listed = voice.voices().await.unwrap().unwrap();
    assert_eq!(listed, ["cow", "calf", "bull"]);
    assert!(voice.probe().await.unwrap().contains("3 voices"));

    let line = |speed: f64| SynthRequest {
        text: "Hello there, cow.".into(),
        locale: "en".into(),
        voice: Some("calf".into()),
        speed,
        instruct: None,
    };
    let said = voice.synthesize(&line(1.0)).await.unwrap();
    assert_eq!(said.pcm.sample_rate, 24_000);
    let ms = said.pcm.duration_ms();
    assert!(
        (1000..3000).contains(&ms),
        "three moos and their pauses: {ms}ms"
    );
    // Twice as fast is half as long.
    let quick = voice
        .synthesize(&line(2.0))
        .await
        .unwrap()
        .pcm
        .duration_ms();
    assert!(quick.abs_diff(ms / 2) < 20, "{quick} vs {ms}");

    // A voice the server lacks is refused in its own words.
    let mut wrong = line(1.0);
    wrong.voice = Some("sheep".into());
    let err = voice.synthesize(&wrong).await.unwrap_err().to_string();
    assert!(err.contains("no voice 'sheep'"), "{err}");
}
