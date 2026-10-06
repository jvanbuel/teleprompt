//! `setup`'s word on the project's voice: whether the server it chose
//! answers, and what is wrong with its `backends:` settings.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use teleprompt_core::config::{PartialConfig, PartialVoice};
use teleprompt_project::project::Project;
use teleprompt_setup::project_voice;

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

/// A `Project` in memory, never written to disk: `project_voice` only
/// ever reads `root` (to build a cache path it does not require to exist)
/// and `config` (for `voice.backend` and `backends`), so a real
/// `teleprompt.toml` and a real directory would test nothing an in-memory
/// value does not.
fn project_with_backend(
    backend_id: &str,
    backends: BTreeMap<String, serde_yaml::Value>,
) -> Project {
    Project {
        registry: teleprompt_registry::registry(),
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

/// `setup` asks the *configured* backend. A freshly scaffolded project's
/// `voice.backend` is `null`, which has no server, so nothing is asked.
#[tokio::test]
async fn a_null_backend_project_probes_nothing() {
    let project = project_with_backend("null", BTreeMap::new());
    let report = project_voice(&project).await;

    assert_eq!(report.backend, "null");
    assert!(
        report.answer.is_none(),
        "a null-backend project has nothing to probe: {:?}",
        report.answer
    );
    assert!(report.problems.is_empty(), "{:?}", report.problems);
}

#[tokio::test]
async fn a_kokoro_backend_project_against_a_reachable_stub_reports_its_voice_count() {
    let stub = kokoro_stub_listing(&["af_heart", "af_bella", "am_adam"]).await;
    let project = project_with_backend("kokoro", kokoro_backends(&stub.base_url));
    let report = project_voice(&project).await;

    let line = report.answer.expect("probe ran");
    assert!(line.contains(&stub.base_url), "{line}");
    assert!(line.contains("reachable"), "{line}");
    assert!(line.contains('3'), "must report the voice count: {line}");
    assert!(
        report.problems.is_empty(),
        "a reachable server is not a problem"
    );
}

#[tokio::test]
async fn a_kokoro_backend_project_against_a_dead_port_is_a_warning_not_a_failure() {
    // Check and plan do not need the server, so setup must not report a red
    // state for a machine that simply has not started it.
    let project = project_with_backend("kokoro", kokoro_backends("http://127.0.0.1:1"));
    let report = project_voice(&project).await;

    let line = report.answer.expect("probe ran");
    assert!(line.to_lowercase().contains("refused"), "{line}");
    assert!(
        report.problems.is_empty(),
        "unreachable is a warning, not an error"
    );
}

/// Accepts a connection, reads the request, then holds the socket open
/// without answering — a server that is up but stuck, the case a dead port
/// (connection refused, instant) does not exercise.
struct HangingKokoroStub {
    base_url: String,
}

async fn kokoro_stub_that_hangs() -> HangingKokoroStub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                // Hold the connection open with no response: reading blocks
                // until the peer (reqwest, once the probe deadline drops its
                // future) gives up.
                let mut sink = [0u8; 1];
                let _ = socket.read(&mut sink).await;
            });
        }
    });
    HangingKokoroStub {
        base_url: format!("http://{addr}"),
    }
}

/// A hanging server used to make the probe wait out the full synthesis
/// `timeout_ms` (30 000 by default) before saying anything, with no word that
/// the failure was specifically a timeout. `timeout_ms` here is set far above
/// the probe's own deadline, so only a probe that gives up on its own — not one
/// that happens to be fast for some other reason — makes this test finish
/// quickly.
#[tokio::test]
async fn a_hanging_kokoro_server_does_not_make_setup_wait_out_the_synthesis_timeout() {
    let stub = kokoro_stub_that_hangs().await;
    let mut backends = kokoro_backends(&stub.base_url);
    backends.insert(
        "kokoro".to_string(),
        serde_yaml::from_str(&format!(
            "base_url: \"{}\"\ntimeout_ms: 30000",
            stub.base_url
        ))
        .unwrap(),
    );
    let project = project_with_backend("kokoro", backends);

    let start = std::time::Instant::now();
    let report = project_voice(&project).await;
    let elapsed = start.elapsed();

    let line = report.answer.expect("probe ran");
    assert!(line.contains("unreachable"), "{line}");
    assert!(
        elapsed < std::time::Duration::from_secs(15),
        "the probe must give up on its own short deadline rather than wait \
         out the 30s synthesis timeout: took {elapsed:?}"
    );
    assert!(
        report.problems.is_empty(),
        "unreachable is a warning, not an error"
    );
}

/// A bad `backends:` value is reported, not silently replaced by defaults:
/// `setup` is the command run when nothing works.
#[tokio::test]
async fn a_bad_backend_setting_is_a_reported_problem_not_a_silent_default() {
    let mut backends = BTreeMap::new();
    backends.insert(
        "kokoro".to_string(),
        serde_yaml::from_str("concurrency: 0").unwrap(),
    );
    let project = project_with_backend("kokoro", backends);
    let report = project_voice(&project).await;

    let joined = report.problems.join("\n");
    assert!(joined.contains("concurrency"), "{joined}");
    assert!(
        report.answer.is_none(),
        "there is no backend to probe: {:?}",
        report.answer
    );
}

/// A misconfigured backend this project does not select stops nothing,
/// but `setup` still says it, because the author wrote that block and
/// expects it to matter.
#[tokio::test]
async fn a_bad_setting_for_an_unselected_backend_is_listed() {
    let mut backends = BTreeMap::new();
    backends.insert(
        "kokoro".to_string(),
        serde_yaml::from_str("concurrency: 0").unwrap(),
    );
    let project = project_with_backend("null", backends);
    let report = project_voice(&project).await;

    assert!(
        report.problems.iter().any(|p| p.contains("concurrency")),
        "{:?}",
        report.problems
    );
}

/// A `backends:` key naming nothing this build ships, and making no server,
/// is reported by `setup` too, and reported even when the project resolves to
/// a different backend entirely — the key belongs to no backend, so
/// "validate only the selected one" must not be a way for it to disappear.
#[tokio::test]
async fn an_unknown_backends_key_is_a_reported_problem() {
    let mut backends = BTreeMap::new();
    backends.insert(
        "kokoro-local".to_string(),
        serde_yaml::from_str("voice: af_heart").unwrap(),
    );
    let project = project_with_backend("null", backends);
    let report = project_voice(&project).await;

    let joined = report.problems.join("\n");
    assert!(joined.contains("backends.kokoro-local"), "{joined}");
}

/// The cache key no longer distinguishes two servers running different
/// weights under one model name — that is the price of a key that travels
/// between machines. `setup` is where the mismatch has to become visible,
/// so the probe line names the model it is talking to, not only the
/// address.
#[tokio::test]
async fn the_probe_line_names_the_model_the_cache_is_keyed_on() {
    let stub = kokoro_stub_listing(&["af_heart", "am_adam"]).await;
    let project = project_with_backend("kokoro", kokoro_backends(&stub.base_url));

    let report = project_voice(&project).await;
    let line = report.answer.expect("a kokoro project probes its server");

    assert!(
        line.contains("kokoro"),
        "the model belongs on the line: {line}"
    );
    assert!(line.contains(&stub.base_url), "{line}");
}

/// Gemini has no voice list to count, so setup asks whether the key opens
/// the model; a missing key names the variable to set.
#[tokio::test]
async fn a_gemini_project_probes_its_key() {
    let backends = |key: &str| {
        BTreeMap::from([(
            "gemini".to_string(),
            serde_yaml::from_str(&format!(
                "base_url: \"http://127.0.0.1:1\"\napi_key_env: \"{key}\""
            ))
            .unwrap(),
        )])
    };
    let project = project_with_backend("gemini", backends("CARGO_PKG_NAME"));
    let report = project_voice(&project).await;
    let line = report.answer.expect("probe ran");
    assert!(line.contains("cannot reach http://127.0.0.1:1"), "{line}");
    assert!(
        report.problems.is_empty(),
        "unreachable is a warning, not an error"
    );

    let project = project_with_backend("gemini", backends("TELEPROMPT_NO_SUCH_KEY_VAR"));
    let report = project_voice(&project).await;
    let line = report.answer.expect("probe ran");
    assert!(line.contains("TELEPROMPT_NO_SUCH_KEY_VAR"), "{line}");
}

/// In a project, `setup` with no tools named says whose voice it is; with
/// tools named, it says only those.
#[test]
fn setup_in_a_project_reports_its_voice() {
    let dir = teleprompt_testkit::test_dir("setup-voice");
    teleprompt_project::new::scaffold(dir.path()).unwrap();
    let setup = |args: &[&str]| -> serde_json::Value {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_teleprompt"))
            .args(["--format", "json", "setup"])
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let all = setup(&[]);
    assert_eq!(all["voice"]["backend"], "null", "{all}");
    assert!(all["voice"]["answer"].is_null(), "{all}");
    assert!(setup(&["ffmpeg"])["voice"].is_null());
}
