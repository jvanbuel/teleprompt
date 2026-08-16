use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use teleprompt_cli::cmd::doctor::doctor_report_with;
use teleprompt_cli::project::Project;
use teleprompt_core::config::{PartialConfig, PartialVoice};
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

/// A `Project` in memory, never written to disk: `doctor_report_with` only
/// ever reads `root` (to build a cache path it does not require to exist)
/// and `config` (for `voice.backend` and `backends`), so a real
/// `teleprompt.toml` and a real directory would test nothing an in-memory
/// value does not.
fn project_with_backend(
    backend_id: &str,
    backends: BTreeMap<String, serde_yaml::Value>,
) -> Project {
    Project {
        root: PathBuf::from("does-not-exist-on-disk"),
        config: PartialConfig {
            voice: Some(PartialVoice {
                backend: Some(backend_id.to_string()),
                ..Default::default()
            }),
            backends: Some(backends),
            ..Default::default()
        },
    }
}

fn kokoro_backends(base_url: &str) -> BTreeMap<String, serde_yaml::Value> {
    let mut backends = BTreeMap::new();
    backends.insert(
        "kokoro".to_string(),
        serde_yaml::from_str(&format!("base_url: \"{base_url}\"")).unwrap(),
    );
    backends
}

/// Spec §9 says `doctor` probes the *configured* backend. A freshly
/// scaffolded project's `voice.backend` is `null`, which has no server —
/// so `doctor` must not reach across the network for it, and must not print
/// a line pretending it did.
#[tokio::test]
async fn a_null_backend_project_probes_nothing() {
    let project = project_with_backend("null", BTreeMap::new());
    let report = doctor_report_with(&SceneRegistry::with_builtins(), Some(&project)).await;

    assert!(
        report.voice_probe.is_none(),
        "a null-backend project has nothing to probe: {:?}",
        report.voice_probe
    );
    let rendered = report.render();
    assert!(
        !rendered.contains("voice kokoro"),
        "no probe line at all, not even an empty one: {rendered}"
    );
    assert!(report.ok);
}

#[tokio::test]
async fn a_kokoro_backend_project_against_a_reachable_stub_reports_its_voice_count() {
    let stub = kokoro_stub_listing(&["af_heart", "af_bella", "am_adam"]).await;
    let project = project_with_backend("kokoro", kokoro_backends(&stub.base_url));
    let report = doctor_report_with(&SceneRegistry::with_builtins(), Some(&project)).await;

    let line = report.voice_probe.expect("probe ran");
    assert!(line.contains(&stub.base_url), "{line}");
    assert!(line.contains("reachable"), "{line}");
    assert!(line.contains('3'), "must report the voice count: {line}");
    assert!(report.ok, "a reachable server is not a problem");
}

#[tokio::test]
async fn a_kokoro_backend_project_against_a_dead_port_is_a_warning_not_a_failure() {
    // Spec §9: check and plan do not need the server, so doctor must not
    // report a red state for a machine that simply has not started it.
    let project = project_with_backend("kokoro", kokoro_backends("http://127.0.0.1:1"));
    let report = doctor_report_with(&SceneRegistry::with_builtins(), Some(&project)).await;

    let line = report.voice_probe.expect("probe ran");
    assert!(line.contains("unreachable"), "{line}");
    assert!(report.ok, "unreachable is a warning, not an error");
}

/// Outside a project there is no configured backend to probe — nothing
/// resolved `voice.backend`, so there is no server that this run could
/// meaningfully be asking about. `doctor` still reports everything else
/// (the fallback cache path, the available adapters and backends); only
/// the probe line is absent.
#[tokio::test]
async fn outside_a_project_there_is_no_probe() {
    let report = doctor_report_with(&SceneRegistry::with_builtins(), None).await;
    assert!(report.voice_probe.is_none(), "{:?}", report.voice_probe);
    assert!(!report.render().contains("voice kokoro"));
    assert!(report.ok);
}
