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

    let probe = report.voice_probe.expect("probe ran");
    let line = &probe.detail;
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

    let probe = report.voice_probe.expect("probe ran");
    let line = &probe.detail;
    assert!(line.contains("unreachable"), "{line}");
    assert!(report.ok, "unreachable is a warning, not an error");
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

/// M2: a hanging server used to make `doctor` wait out the full synthesis
/// `timeout_ms` (30 000 by default) before saying anything, with no word
/// that the failure was specifically a timeout. `timeout_ms` here is set
/// far above the probe's own deadline, so only a probe that gives up on its
/// own — not one that happens to be fast for some other reason — makes this
/// test finish quickly.
#[tokio::test]
async fn a_hanging_kokoro_server_does_not_make_doctor_wait_out_the_synthesis_timeout() {
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
    let report = doctor_report_with(&SceneRegistry::with_builtins(), Some(&project)).await;
    let elapsed = start.elapsed();

    let probe = report.voice_probe.expect("probe ran");
    assert!(probe.detail.contains("unreachable"), "{}", probe.detail);
    assert!(
        elapsed < std::time::Duration::from_secs(15),
        "the probe must give up on its own short deadline rather than wait \
         out the 30s synthesis timeout: took {elapsed:?}"
    );
    assert!(report.ok, "unreachable is a warning, not an error");
}

/// The probe line names the backend it probed. `render` used to print the
/// literal `voice kokoro` for whatever backend was configured — invisible
/// today, and the moment a second server-backed backend exists it labels one
/// machine's answer with the other one's name. Asserted on a report built by
/// hand, because a second server-backed backend is exactly what this build
/// does not have.
#[test]
fn the_probe_line_names_the_backend_that_was_probed() {
    let report = teleprompt_cli::cmd::doctor::DoctorReport {
        ok: true,
        adapters: Vec::new(),
        capture_backends: Vec::new(),
        voice_backends: Vec::new(),
        manifest_version: 1,
        cache_root: ".teleprompt/cache".to_string(),
        cache_entries: 0,
        cache_bytes: 0,
        compose_entries: 0,
        compose_bytes: 0,
        ffmpeg: None,
        voice_probe: Some(teleprompt_cli::cmd::doctor::VoiceProbe {
            backend: "elevenlabs".to_string(),
            detail: "https://api.example — reachable, 2 voices".to_string(),
        }),
        problems: Vec::new(),
        notes: Vec::new(),
    };
    let rendered = report.render();
    assert!(rendered.contains("voice elevenlabs"), "{rendered}");
    assert!(!rendered.contains("kokoro"), "{rendered}");
}

/// I2's third leg. `doctor` used to catch a bad `backends:` value, silently
/// substitute defaults and report a healthy project — the command you run
/// when nothing works, hiding the thing that is breaking your build. Its
/// comment claimed `check` reported it "with a shot"; `check` reported it
/// as a bare line with no file at all, and on a `null` project reported it
/// for a backend that project never uses.
#[tokio::test]
async fn a_bad_backend_setting_is_a_reported_problem_not_a_silent_default() {
    let mut backends = BTreeMap::new();
    backends.insert(
        "kokoro".to_string(),
        serde_yaml::from_str("concurrency: 0").unwrap(),
    );
    let project = project_with_backend("kokoro", backends);
    let report = doctor_report_with(&SceneRegistry::with_builtins(), Some(&project)).await;

    let joined = report.problems.join("\n");
    assert!(joined.contains("concurrency"), "{joined}");
    assert!(
        !report.ok,
        "a project that cannot build its backend is not ok"
    );
    assert!(
        report.render().contains("concurrency"),
        "{}",
        report.render()
    );
    assert!(
        report.voice_probe.is_none(),
        "there is no backend to probe: {:?}",
        report.voice_probe
    );
    assert!(
        report.voice_backends.contains(&"kokoro".to_string()),
        "a misconfigured backend is still one this build ships: {:?}",
        report.voice_backends
    );
}

/// The other side of I2's fix. A misconfigured backend this project does
/// not select stops nothing, so `ok` stays true — but `doctor` still says
/// it, because the author wrote that block and expects it to matter.
#[tokio::test]
async fn a_bad_setting_for_an_unselected_backend_is_listed_without_turning_the_report_red() {
    let mut backends = BTreeMap::new();
    backends.insert(
        "kokoro".to_string(),
        serde_yaml::from_str("concurrency: 0").unwrap(),
    );
    let project = project_with_backend("null", backends);
    let report = doctor_report_with(&SceneRegistry::with_builtins(), Some(&project)).await;

    assert!(
        report.problems.iter().any(|p| p.contains("concurrency")),
        "{:?}",
        report.problems
    );
    assert!(
        report.ok,
        "a backend this project never selects blocks nothing"
    );
}

/// I1's other surface. A `backends:` key naming nothing this build ships is
/// reported by `doctor` too, and reported even when the project resolves to
/// a different backend entirely — the key belongs to no backend, so
/// "validate only the selected one" must not be a way for it to disappear.
#[tokio::test]
async fn an_unknown_backends_key_is_a_reported_problem() {
    let mut backends = BTreeMap::new();
    backends.insert(
        "kokoro-local".to_string(),
        serde_yaml::from_str("base_url: \"http://127.0.0.1:8881\"").unwrap(),
    );
    let project = project_with_backend("null", backends);
    let report = doctor_report_with(&SceneRegistry::with_builtins(), Some(&project)).await;

    let joined = report.problems.join("\n");
    assert!(joined.contains("backends.kokoro-local"), "{joined}");
    assert!(!report.ok);
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

/// Spec §9: a missing or too-old ffmpeg is the most likely first-run
/// failure, so `doctor` is where it is found rather than fifteen minutes
/// into a render.
#[tokio::test]
async fn the_report_says_whether_this_machine_can_render() {
    let report = doctor_report_with(&SceneRegistry::with_builtins(), None).await;

    let has_ffmpeg = std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok();
    assert_eq!(
        report.ffmpeg.is_some(),
        has_ffmpeg,
        "the probe agrees with the machine it ran on"
    );

    let rendered = report.render();
    assert!(
        rendered.contains("ffmpeg"),
        "and it is in the human report either way: {rendered}"
    );
    assert!(
        !rendered.contains("M0 builds no video"),
        "the note that video was out of scope outlived its milestone: {rendered}"
    );
}

/// A version string comes from the binary, never from a guess.
#[test]
fn a_binary_that_is_not_there_probes_to_nothing() {
    assert_eq!(
        teleprompt_cli::cmd::doctor::probe_ffmpeg("teleprompt-no-such-binary"),
        None
    );
}

/// The cache key no longer distinguishes two servers running different
/// weights under one model name — that is the price of a key that travels
/// between machines. `doctor` is where the mismatch has to become visible,
/// so the probe line names the model it is talking to, not only the
/// address.
#[tokio::test]
async fn the_probe_line_names_the_model_the_cache_is_keyed_on() {
    let stub = kokoro_stub_listing(&["af_heart", "am_adam"]).await;
    let project = project_with_backend("kokoro", kokoro_backends(&stub.base_url));

    let report = doctor_report_with(&SceneRegistry::with_builtins(), Some(&project)).await;
    let probe = report
        .voice_probe
        .expect("a kokoro project probes its server");

    assert!(
        probe.detail.contains("kokoro"),
        "the model belongs on the line: {}",
        probe.detail
    );
    assert!(probe.detail.contains(&stub.base_url), "{}", probe.detail);
}
