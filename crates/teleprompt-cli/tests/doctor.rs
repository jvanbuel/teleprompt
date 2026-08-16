use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use teleprompt_cli::cmd::doctor::doctor_report_with;
use teleprompt_scene::SceneRegistry;

/// A minimal `/v1/audio/voices` responder, built the same way
/// `tests/dub.rs`'s `kokoro_stub_listing` is — copied rather than shared
/// across the two test binaries for the same reason that one gives: the
/// ~20 lines a single-shape JSON GET responder needs are not worth a
/// shared test-support crate for two call sites.
struct KokoroStub {
    base_url: String,
}

async fn kokoro_stub_listing(voices: &[&str]) -> KokoroStub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let body = serde_json::json!({ "voices": voices }).to_string();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            requests.fetch_add(1, Ordering::SeqCst);
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: \
                     application/json\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });

    KokoroStub {
        base_url: format!("http://{addr}"),
    }
}

/// `doctor_report_with` against a `backends:` map pointing kokoro at `url`
/// — the seam that stands in for the brief's undefined `doctor_report_for`.
async fn report_for_kokoro_url(url: &str) -> teleprompt_cli::cmd::doctor::DoctorReport {
    let mut backends = BTreeMap::new();
    backends.insert(
        "kokoro".to_string(),
        serde_yaml::from_str(&format!("base_url: \"{url}\"")).unwrap(),
    );
    doctor_report_with(&SceneRegistry::with_builtins(), &backends).await
}

#[tokio::test]
async fn doctor_reports_a_reachable_server_with_its_voice_count() {
    let stub = kokoro_stub_listing(&["af_heart", "af_bella", "am_adam"]).await;
    let report = report_for_kokoro_url(&stub.base_url).await;
    let line = report.voice_probe.expect("probe ran");
    assert!(line.contains(&stub.base_url), "{line}");
    assert!(line.contains("reachable"), "{line}");
    assert!(line.contains('3'), "must report the voice count: {line}");
    assert!(report.ok, "a reachable server is not a problem");
}

#[tokio::test]
async fn an_unreachable_server_is_a_warning_not_a_failure() {
    // Spec §9: check and plan do not need the server, so doctor must not
    // report a red state for a machine that simply has not started it.
    let report = report_for_kokoro_url("http://127.0.0.1:1").await;
    let line = report.voice_probe.expect("probe ran");
    assert!(line.contains("unreachable"), "{line}");
    assert!(report.ok, "unreachable is a warning, not an error");
}
