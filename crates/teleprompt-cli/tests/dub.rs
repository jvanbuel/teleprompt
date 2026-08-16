use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use teleprompt_cli::cmd::dub::{run_dub, DubError};
use teleprompt_cli::project::Project;

const SCRIPT: &str = "\
# Quick start

Every video in this repository is built from a script you can read.

# Provenance

And every timeline is committed alongside it.
";

fn tempdir(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "teleprompt-dub-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

/// Scaffold a project and drop `script` at `scripts/test.md`, mirroring
/// `commands.rs::project_with`. Returns the project root.
fn project_with(tag: &str, script: &str) -> PathBuf {
    let dir = tempdir(tag);
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(dir.join("scripts/test.md"), script).unwrap();
    Project::discover(&dir).unwrap();
    dir
}

fn tp(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("process exited normally")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn read_manifest(root: &Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(root.join("public/narration/en/narration.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn dub_writes_a_manifest_and_one_wav_per_segment() {
    let root = project_with("write", SCRIPT);
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    let m = read_manifest(&root);
    assert_eq!(m["manifest_version"], 1);
    assert_eq!(m["locale"], "en");
    assert_eq!(m["segments"].as_array().unwrap().len(), 2);

    for seg in m["segments"].as_array().unwrap() {
        let rel = seg["audio"].as_str().unwrap();
        let wav = root.join("public/narration/en").join(rel);
        assert!(wav.exists(), "missing {rel}");
        assert_eq!(&std::fs::read(&wav).unwrap()[0..4], b"RIFF");
    }
}

/// Length in milliseconds of a written WAV's `data` chunk, derived from the
/// manifest's own `audio` block rather than from hardcoded constants: a
/// hardcoded `/ 2 * 1000 / 48_000` keeps passing when the sample rate or
/// channel count changes, which is precisely the drift this asserts against.
fn wav_ms(root: &Path, m: &serde_json::Value, seg: &serde_json::Value) -> u64 {
    let sample_rate = m["audio"]["sample_rate"].as_u64().unwrap();
    let channels = m["audio"]["channels"].as_u64().unwrap();
    // 16-bit PCM: two bytes per sample, `channels` samples per frame.
    let bytes_per_frame = 2 * channels;

    let bytes = wav_bytes(root, seg);
    assert_eq!(&bytes[0..4], b"RIFF");
    let data_len = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as u64;
    data_len / bytes_per_frame * 1000 / sample_rate
}

fn wav_bytes(root: &Path, seg: &serde_json::Value) -> Vec<u8> {
    std::fs::read(
        root.join("public/narration/en")
            .join(seg["audio"].as_str().unwrap()),
    )
    .unwrap()
}

#[test]
fn the_wav_length_matches_the_duration_the_manifest_claims() {
    let root = project_with("length", SCRIPT);
    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );

    let m = read_manifest(&root);
    for seg in m["segments"].as_array().unwrap() {
        assert_eq!(
            wav_ms(&root, &m, seg),
            seg["duration_ms"].as_u64().unwrap(),
            "a consumer placing `{}` at its stated duration must not clip it",
            seg["id"].as_str().unwrap()
        );
    }
}

/// C1. `dub` used to build its own `SynthRequest` with `voice: None,
/// speed: 1.0`, while the duration in the manifest came from the segment's
/// *resolved* config. With `speed: 2.0` the manifest published half the
/// length of the file that sat beside it.
#[test]
fn a_non_default_speed_renders_audio_the_manifest_agrees_with() {
    let root = project_with(
        "speed",
        "\
---
voice:
  speed: 2.0
---

# Quick start

Every video in this repository is built from a script you can read.
",
    );
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    let m = read_manifest(&root);
    let seg = &m["segments"][0];
    let claimed = seg["duration_ms"].as_u64().unwrap();

    // The speed actually took effect, so this is not passing by accident on
    // two default-speed renders.
    assert_eq!(
        claimed, 2775,
        "5550ms of speech at speed 2.0; if this changes the fixture drifted"
    );
    assert_eq!(
        wav_ms(&root, &m, seg),
        claimed,
        "the WAV must be rendered from the same request the duration was \
         measured from"
    );
}

/// I4. The manifest's `audio_hash` must describe the file on disk, per spec
/// §5.1 — that is what lets a consumer skip re-encoding a byte-identical
/// render. It is a different quantity from the timeline's `audio_hash`,
/// which identifies *which* audio a segment resolves to and correctly does
/// not move when `dub` re-renders the same bytes. `manifest::build` can only
/// seed this field with the timeline's value, so the overwrite in `dub` is
/// what makes the published field mean what it says.
#[test]
fn audio_hash_is_the_hash_of_the_bytes_on_disk() {
    let root = project_with("audiohash", SCRIPT);
    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );

    let m = read_manifest(&root);
    for seg in m["segments"].as_array().unwrap() {
        let bytes = wav_bytes(&root, seg);
        let expected = teleprompt_core::Hash::of(&bytes).to_string();
        assert_eq!(
            seg["audio_hash"].as_str().unwrap(),
            expected,
            "`{}`: spec §5.1 documents audio_hash as hashing the rendered file",
            seg["id"].as_str().unwrap()
        );
    }
}

#[test]
fn audio_hash_is_identical_across_two_runs() {
    let root = project_with("audiohashstable", SCRIPT);
    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    let first: Vec<String> = read_manifest(&root)["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["audio_hash"].as_str().unwrap().to_string())
        .collect();

    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    let second: Vec<String> = read_manifest(&root)["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["audio_hash"].as_str().unwrap().to_string())
        .collect();

    assert_eq!(first, second);
    assert!(!first.is_empty());
}

/// I1. A segment id becomes `audio/<id>.wav`, so an explicit `..` id used to
/// write outside `--out` and publish an escaping path in the manifest.
#[test]
fn an_id_that_escapes_the_output_directory_is_rejected_before_anything_is_written() {
    let root = project_with("traversal", "# A\n\nOne. {#../../../pwned}\n");
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );

    assert_eq!(code(&out), 2, "an unusable id is a validation error");
    assert!(
        !root.join("public/narration/en").exists(),
        "no output at all, let alone outside --out"
    );
    assert!(
        !root.join("pwned.wav").exists() && !root.parent().unwrap().join("pwned.wav").exists(),
        "nothing may be written outside --out"
    );
    // And `check` catches it too, so an author does not first learn of it
    // from a file in the wrong directory.
    assert_eq!(code(&tp(&root, &["check", "scripts/test.md"])), 2);
}

/// The other side of I1: the hazard is traversal and control characters,
/// not non-ASCII letters. teleprompt is localization-first, so a heading in
/// a language it exists to dub must produce a working segment, a working
/// file, and a working manifest path — end to end, not just past
/// `assign_ids`.
#[test]
fn a_non_ascii_heading_dubs_to_a_real_file() {
    let root = project_with("unicode", "# Café\n\nUn café, s'il vous plaît.\n");
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    let m = read_manifest(&root);
    let seg = &m["segments"][0];
    assert_eq!(seg["id"], "café-1");
    assert_eq!(seg["audio"], "audio/café-1.wav");
    assert_eq!(seg["chapter"], "café");

    let wav = root
        .join("public/narration/en")
        .join(seg["audio"].as_str().unwrap());
    assert!(wav.exists(), "{}", wav.display());
    assert_eq!(&std::fs::read(&wav).unwrap()[0..4], b"RIFF");
    assert_eq!(wav_ms(&root, &m, seg), seg["duration_ms"].as_u64().unwrap());

    // Still exactly one path segment below `audio/`.
    assert!(wav.starts_with(root.join("public/narration/en/audio")));
}

const RECORDED: &str = "\
---
voice:
  source: recorded
---

# Quick start

Every video in this repository is built from a script you can read.
";

/// I3. M0 has no recorder, so `source: recorded` downgrades to `synthetic`.
/// That is exercisable today, contrary to an earlier claim that downgrades
/// could not be reached in M0.
#[test]
fn a_recorded_request_downgrades_and_the_manifest_says_so() {
    let root = project_with("downgrade", RECORDED);
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    assert_eq!(code(&out), 0, "a downgrade is not fatal by default");

    let m = read_manifest(&root);
    let seg = &m["segments"][0];
    assert_eq!(seg["voice_source"], "recorded");
    assert_eq!(seg["voice_source_actual"], "synthetic");
    assert!(
        seg["downgrade_reason"].is_string(),
        "a downgrade without a reason is unactionable: {}",
        seg["downgrade_reason"]
    );
}

#[test]
fn strict_voice_makes_a_downgrade_fatal() {
    let root = project_with("strict", RECORDED);
    let out = tp(
        &root,
        &[
            "dub",
            "scripts/test.md",
            "--out",
            "public/narration",
            "--strict-voice",
        ],
    );

    assert_eq!(
        code(&out),
        4,
        "spec §3: downgrades are fatal under --strict-voice"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("quick-start-1") && stderr.contains("recorded"),
        "the report must name which segments downgraded and why: {stderr}"
    );
}

#[test]
fn strict_voice_is_silent_when_every_segment_got_the_tier_it_asked_for() {
    let root = project_with("strictclean", SCRIPT);
    let out = tp(
        &root,
        &[
            "dub",
            "scripts/test.md",
            "--out",
            "public/narration",
            "--strict-voice",
        ],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn dub_is_byte_stable_across_runs() {
    let root = project_with("stable", SCRIPT);
    tp(&root, &["dub", "scripts/test.md", "--out", "a"]);
    tp(&root, &["dub", "scripts/test.md", "--out", "b"]);

    let a = std::fs::read(root.join("a/en/narration.json")).unwrap();
    let b = std::fs::read(root.join("b/en/narration.json")).unwrap();
    assert_eq!(a, b, "a committed manifest must not churn between runs");
}

#[test]
fn check_passes_on_a_fresh_manifest_and_writes_nothing() {
    let root = project_with("checkclean", SCRIPT);
    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );

    let before = std::fs::read(root.join("public/narration/en/narration.json")).unwrap();
    let out = tp(
        &root,
        &[
            "dub",
            "scripts/test.md",
            "--out",
            "public/narration",
            "--check",
        ],
    );
    let after = std::fs::read(root.join("public/narration/en/narration.json")).unwrap();

    assert_eq!(code(&out), 0, "{}", stdout(&out));
    assert_eq!(before, after, "--check must not write");
}

#[test]
fn check_exits_3_when_the_prose_moved_on() {
    let root = project_with("drift", SCRIPT);
    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );

    std::fs::write(
        root.join("scripts/test.md"),
        SCRIPT.replace(
            "And every timeline is committed alongside it.",
            "And every timeline is committed alongside it, which is what makes \
             the whole review story work at all.",
        ),
    )
    .unwrap();

    let out = tp(
        &root,
        &[
            "dub",
            "scripts/test.md",
            "--out",
            "public/narration",
            "--check",
        ],
    );
    assert_eq!(code(&out), 3, "stale manifest must fail CI");
    assert!(stdout(&out).contains("text edited"), "{}", stdout(&out));
}

#[test]
fn check_exits_3_when_no_manifest_has_been_written_at_all() {
    let root = project_with("nomanifest", SCRIPT);
    let out = tp(
        &root,
        &[
            "dub",
            "scripts/test.md",
            "--out",
            "public/narration",
            "--check",
        ],
    );
    assert_eq!(
        code(&out),
        3,
        "a missing manifest is maximal drift, not a crash"
    );
}

#[test]
fn a_manifest_from_a_future_version_is_refused_rather_than_misread() {
    let root = project_with("future", SCRIPT);
    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );

    let path = root.join("public/narration/en/narration.json");
    let raw = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        raw.replace("\"manifest_version\": 1", "\"manifest_version\": 999"),
    )
    .unwrap();

    let out = tp(
        &root,
        &[
            "dub",
            "scripts/test.md",
            "--out",
            "public/narration",
            "--check",
        ],
    );
    assert_eq!(
        code(&out),
        1,
        "an unreadable manifest is a runtime failure, not drift"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("manifest_version"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_broken_script_fails_validation_before_writing_anything() {
    let root = project_with("broken", "# Intro\n\nOne. {#a polcy=hold}\n");
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );

    assert_eq!(code(&out), 2);
    assert!(
        !root.join("public/narration/en").exists(),
        "no partial output"
    );
}

/// Overwrite every sidecar in the project's cache with garbage. Returns how
/// many were clobbered, so a test cannot silently pass against an empty
/// cache.
fn corrupt_every_sidecar(root: &Path) -> usize {
    let dir = root.join(".teleprompt/cache/voice");
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).expect("dub must have populated the cache") {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            std::fs::write(&path, "{ not json").unwrap();
            n += 1;
        }
    }
    n
}

/// I4. A corrupt sidecar used to exit 2 out of `check`, attributed to the
/// script — a derived, gitignored artifact bricking a validation command,
/// with no recovery path offered and no `cache clean` to offer. The cache is
/// content-addressed and self-healing, so the only correct reading is a
/// miss.
#[test]
fn a_corrupt_cache_entry_reads_as_a_miss_rather_than_bricking_check() {
    let root = project_with("corruptcheck", SCRIPT);
    let dubbed = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    assert_eq!(
        code(&dubbed),
        0,
        "{}",
        String::from_utf8_lossy(&dubbed.stderr)
    );
    assert!(corrupt_every_sidecar(&root) > 0);

    let out = tp(&root, &["check", "scripts/test.md"]);
    assert_eq!(
        code(&out),
        0,
        "a corrupt cache entry must not fail validation: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning:") && stderr.contains(".teleprompt"),
        "the author must be told which file caused the re-synthesis: {stderr}"
    );
}

/// And the entry heals: `dub` re-renders the segment and publishes a
/// measurement, rather than either failing or quietly shipping an estimate.
#[test]
fn a_corrupt_cache_entry_is_re_rendered_by_dub() {
    let root = project_with("corruptdub", SCRIPT);
    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    let before = read_manifest(&root);
    assert!(corrupt_every_sidecar(&root) > 0);

    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("warning:"),
        "a silent re-render leaves the author guessing why dub got slow"
    );

    let after = read_manifest(&root);
    assert_eq!(
        before, after,
        "a re-rendered segment must reproduce byte-identically — that is what \
         content-addressing buys"
    );
    for seg in after["segments"].as_array().unwrap() {
        assert_eq!(seg["duration_source"], "measured");
    }
}

// --- Spec §7: the voice list is checked once, before the first segment is
// synthesized. The tests below call `run_dub` in-process rather than through
// `tp` (the `teleprompt` binary), because the whole point is to observe
// network traffic (or its absence) mid-command — a subprocess only ever
// hands back an exit code and captured output.

/// A discovered project plus the two paths `run_dub`/`run_dub_with` want
/// directly, for tests that call them in-process.
struct TestProject {
    project: Project,
    script: PathBuf,
    out: PathBuf,
}

/// Scaffold a project whose `teleprompt.toml` is replaced by `config_toml`
/// (verbatim TOML — the project's own `backends:` settings are what
/// `run_dub` builds its registry from, before the script is ever read; see
/// `compile_script`'s doc comment), and drop `script` at `scripts/test.md`.
fn project_with_config_and_script(config_toml: &str, script: &str) -> TestProject {
    let dir = tempdir("inprocess");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    if !config_toml.is_empty() {
        std::fs::write(dir.join("teleprompt.toml"), config_toml).unwrap();
    }
    std::fs::write(dir.join("scripts/test.md"), script).unwrap();
    let project = Project::discover(&dir).unwrap();
    TestProject {
        script: dir.join("scripts/test.md"),
        out: dir.join("public/narration"),
        project,
    }
}

/// [`project_with_config_and_script`] with the project's scaffolded default
/// config (`null` backend) left untouched.
fn project_with_script(script: &str) -> TestProject {
    project_with_config_and_script("", script)
}

/// A minimal `/v1/audio/voices` responder, built the same way as
/// `teleprompt-voice-kokoro`'s `tests/stub/mod.rs`. Copied rather than
/// imported across the crate boundary — the ~20 lines a single-shape JSON
/// GET responder needs are not worth a shared test-support crate for two
/// call sites, and this one only ever needs the one reply shape `voices()`
/// asks for, not that module's whole `Reply` enum.
struct KokoroStub {
    base_url: String,
    requests: Arc<AtomicUsize>,
}

impl KokoroStub {
    fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

async fn kokoro_stub_listing(voices: &[&str]) -> KokoroStub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let body = serde_json::json!({ "voices": voices }).to_string();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            seen.fetch_add(1, Ordering::SeqCst);
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
        requests,
    }
}

/// Regression guard, not new coverage: this already passed before the
/// voice-list check landed, because `null` never had a network call to
/// skip. It stays here to catch the day the guard added for kokoro becomes
/// a probe every backend pays for — `null` has no server, so a probe here
/// would either fail (nothing listening) or hang, and this test would
/// notice either way.
#[tokio::test]
async fn dub_with_the_null_backend_makes_no_network_call() {
    let p = project_with_script("# Intro\n\nHello there.\n");
    run_dub(&p.project, &p.script, "en", &p.out, false)
        .await
        .unwrap_or_else(|e| match e {
            DubError::Validation(v) => panic!("validation: {v:?}"),
            DubError::Runtime(r) => panic!("runtime: {r}"),
        });
}

/// Spec §7: an unknown voice fails on segment zero, not after the
/// twentieth. The stub's request count is the proof — one request for the
/// voice list, and nothing for synthesis.
#[tokio::test]
async fn an_unknown_kokoro_voice_fails_before_any_segment_is_synthesized() {
    let stub = kokoro_stub_listing(&["af_heart", "af_bella"]).await;
    let p = project_with_config_and_script(
        &format!(
            "[voice]\nbackend = \"kokoro\"\nvoice = \"nonexistent\"\n\n[backends.kokoro]\n\
             base_url = \"{}\"\n",
            stub.base_url
        ),
        "# Intro\n\nHello there.\n",
    );

    let result = run_dub(&p.project, &p.script, "en", &p.out, false).await;
    let msgs = match result {
        Ok(_) => panic!("an unknown voice must fail validation, not synthesize"),
        Err(DubError::Validation(msgs)) => msgs,
        Err(DubError::Runtime(r)) => {
            panic!("must be a validation error, not a runtime failure: {r}")
        }
    };
    let joined = msgs.join("\n");
    assert!(joined.contains("nonexistent"), "{joined}");
    assert!(
        joined.contains("af_heart"),
        "must list what is available: {joined}"
    );

    // Nothing was synthesized: the stub saw the voice-list request and
    // nothing else.
    assert_eq!(
        stub.request_count(),
        1,
        "must fail before the first segment is rendered"
    );
}
