use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use teleprompt_cli::cmd::dub::{manifest_path, run_dub, DubError};
use teleprompt_cli::project::Project;
use teleprompt_manifest::MANIFEST_VERSION;

const SCRIPT: &str = "\
# Quick start

Every video in this repository is built from a script you can read.

# Provenance

And every timeline is committed alongside it.
";

fn tempdir(tag: &str) -> teleprompt_testkit::TestDir {
    teleprompt_testkit::test_dir(&format!("dub-{tag}"))
}

/// Scaffold a project and drop `script` at `scripts/test.md`, mirroring
/// `commands.rs::project_with`. Returns the project root.
fn project_with(tag: &str, script: &str) -> teleprompt_testkit::TestDir {
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

/// Captions sit beside the manifest, one cue or more per line, in both
/// formats players read.
#[test]
fn dub_writes_captions_beside_the_manifest() {
    let root = project_with("captions", SCRIPT);
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let dir = root.join("public/narration/en");
    let chapters = std::fs::read_to_string(dir.join("chapters.txt")).unwrap();
    assert!(chapters.starts_with("0:00 Quick start\n"), "{chapters}");
    let vtt = std::fs::read_to_string(dir.join("captions.vtt")).unwrap();
    let srt = std::fs::read_to_string(dir.join("captions.srt")).unwrap();
    assert!(vtt.starts_with("WEBVTT\n\n00:00:"), "{vtt}");
    assert!(srt.starts_with("1\n00:00:"), "{srt}");
    let m = read_manifest(&root);
    for line in m["lines"].as_array().unwrap() {
        let first = line["text"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        assert!(vtt.contains(first), "{first} in {vtt}");
    }
}

#[test]
fn dub_writes_a_manifest_and_one_wav_per_line() {
    let root = project_with("write", SCRIPT);
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    let m = read_manifest(&root);
    assert_eq!(m["manifest_version"], MANIFEST_VERSION);
    assert_eq!(m["locale"], "en");
    assert_eq!(m["lines"].as_array().unwrap().len(), 2);

    for seg in m["lines"].as_array().unwrap() {
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
    for seg in m["lines"].as_array().unwrap() {
        assert_eq!(
            wav_ms(&root, &m, seg),
            seg["duration_ms"].as_u64().unwrap(),
            "a consumer placing `{}` at its stated duration must not clip it",
            seg["id"].as_str().unwrap()
        );
    }
}

/// C1. `dub` used to build its own `SynthRequest` with `voice: None,
/// speed: 1.0`, while the duration in the manifest came from the line's
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
    let seg = &m["lines"][0];
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

/// The manifest's `audio_hash` must describe the file on disk
/// (docs/design.md#manifest) — that is what lets a consumer skip re-encoding a
/// byte-identical render. It is a different quantity from the timeline's
/// `audio_hash`, which identifies *which* audio a line resolves to and
/// correctly does not move when `dub` re-renders the same bytes.
/// `manifest::build` can only seed this field with the timeline's value, so the
/// overwrite in `dub` is what makes the published field mean what it says.
#[test]
fn audio_hash_is_the_hash_of_the_bytes_on_disk() {
    let root = project_with("audiohash", SCRIPT);
    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );

    let m = read_manifest(&root);
    for seg in m["lines"].as_array().unwrap() {
        let bytes = wav_bytes(&root, seg);
        let expected = teleprompt_core::Hash::of(&bytes).to_string();
        assert_eq!(
            seg["audio_hash"].as_str().unwrap(),
            expected,
            "`{}`: audio_hash hashes the rendered file",
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
    let first: Vec<String> = read_manifest(&root)["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["audio_hash"].as_str().unwrap().to_string())
        .collect();

    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    let second: Vec<String> = read_manifest(&root)["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["audio_hash"].as_str().unwrap().to_string())
        .collect();

    assert_eq!(first, second);
    assert!(!first.is_empty());
}

/// A line id becomes `audio/<id>.wav`, so an explicit `..` id used to write
/// outside `--out` and publish an escaping path in the manifest.
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
/// a language it exists to dub must produce a working line, a working
/// file, and a working manifest path — end to end, not just past
/// `check_ids`.
#[test]
fn a_non_ascii_heading_dubs_to_a_real_file() {
    let root = project_with("unicode", "# Café\n\nUn café, s'il vous plaît.\n");
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    let m = read_manifest(&root);
    let seg = &m["lines"][0];
    assert_eq!(seg["id"], "café-1");
    assert_eq!(seg["audio"], "audio/café-1.wav");
    assert_eq!(seg["chapter"], "café");

    let wav = root
        .join("public/narration/en")
        .join(seg["audio"].as_str().unwrap());
    assert!(wav.exists(), "{}", wav.display());
    assert_eq!(&std::fs::read(&wav).unwrap()[0..4], b"RIFF");
    assert_eq!(wav_ms(&root, &m, seg), seg["duration_ms"].as_u64().unwrap());

    // Still exactly one path line below `audio/`.
    assert!(wav.starts_with(root.join("public/narration/en/audio")));
}

/// With no voice tiers nothing can be downgraded, so there is nothing for
/// `--strict-voice` to make fatal, and it is not a flag.
#[test]
fn strict_voice_is_not_a_flag() {
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
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(code(&out), 2, "{stderr}");
    assert!(stderr.contains("--strict-voice"), "{stderr}");
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
    let from = format!("\"manifest_version\": {MANIFEST_VERSION}");
    assert!(
        raw.contains(&from),
        "the manifest should state the version this build writes"
    );
    std::fs::write(&path, raw.replace(&from, "\"manifest_version\": 999")).unwrap();

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

/// A corrupt sidecar used to exit 2 out of `check`, attributed to the script —
/// a derived, gitignored artifact bricking a validation command, with no
/// recovery path offered and no `cache clean` to offer. The cache is
/// content-addressed and self-healing, so the only correct reading is a miss.
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

/// And the entry heals: `dub` re-renders the line and publishes a
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
        "a re-rendered line must reproduce byte-identically — that is what \
         content-addressing buys"
    );
    for seg in after["lines"].as_array().unwrap() {
        assert_eq!(seg["duration_source"], "measured");
    }
}

/// The `null` backend has no `concurrency` of its own, so `limit = 1`: the
/// semaphore admits one task at a time, and on the CLI's current-thread
/// runtime that makes spawn order, poll order, acquire order, and
/// completion order all the same chain. That chain must start in document
/// order — lines are grouped by cache key before being spawned, and an
/// earlier version of that grouping iterated a `HashMap`'s values, which is
/// a fresh random permutation every process, so progress on the *default*
/// backend printed a different, non-reproducible line order on every
/// run. Real subprocess invocations, parsing the actual `[n/total] <id>
/// done` lines emitted on stderr — not a single pass, and not an assertion
/// on the *set* of ids, both of which would pass just as happily against a
/// random permutation.
#[test]
fn null_path_progress_is_document_order_on_every_run() {
    let root = project_with(
        "progressorder",
        "# Lines\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n\nSix.\n",
    );
    let expected: Vec<String> = (1..=6).map(|n| format!("lines-{n}")).collect();

    for run in 0..20 {
        let out = tp(
            &root,
            &["dub", "scripts/test.md", "--out", "public/narration"],
        );
        assert_eq!(
            code(&out),
            0,
            "run {run}: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let stderr = String::from_utf8_lossy(&out.stderr);
        let ids: Vec<String> = stderr
            .lines()
            .filter_map(|line| {
                let rest = line.trim().strip_prefix('[')?;
                let (_, after_bracket) = rest.split_once(']')?;
                after_bracket
                    .trim()
                    .strip_suffix(" done")
                    .map(str::to_string)
            })
            .collect();

        assert_eq!(
            ids, expected,
            "run {run}: progress must print in document order on the serial \
             (null, limit=1) path every time; a HashMap-ordered spawn would \
             show a different permutation on some runs: {ids:?}"
        );
    }
}

// --- The voice list is checked once, before the first line is synthesized. The
// tests below call `run_dub` in-process rather than through `tp` (the
// `teleprompt` binary), because the whole point is to observe network traffic
// (or its absence) mid-command — a subprocess only ever hands back an exit code
// and captured output.

/// A discovered project plus the two paths `run_dub`/`run_dub_with` want
/// directly, for tests that call them in-process.
struct TestProject {
    _dir: teleprompt_testkit::TestDir,
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
        _dir: dir,
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

/// An unknown voice fails on line zero, not after the twentieth. The stub's
/// request count is the proof — one request for the voice list, and nothing for
/// synthesis.
#[tokio::test]
async fn an_unknown_kokoro_voice_fails_before_any_line_is_synthesized() {
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
        "must fail before the first line is rendered"
    );
}

/// A server that is unreachable, slow, or returns non-200 fails the command
/// with exit 1 — a fact about the machine, not the script. This pins the other
/// side of the split the test above pins: that one asserts `Validation` when
/// the server answers and simply does not list the configured voice; this one
/// asserts `Runtime` when the server never answers at all. A version that
/// collapsed both `kokoro.voices()` failure modes into one `DubError` variant
/// would pass only one of the two tests, depending on which way it collapsed —
/// checking just one arm would not catch that.
#[tokio::test]
async fn an_unreachable_kokoro_server_fails_as_a_runtime_error_not_validation() {
    // Port 1 on loopback: nothing listens, connection refused immediately —
    // the same fixture `teleprompt-voice-kokoro`'s own
    // `an_unreachable_server_names_the_url` uses, for the same reason (fast
    // and deterministic, no timeout to wait out).
    let p = project_with_config_and_script(
        "[voice]\nbackend = \"kokoro\"\nvoice = \"af_heart\"\n\n[backends.kokoro]\n\
         base_url = \"http://127.0.0.1:1\"\n",
        "# Intro\n\nHello there.\n",
    );

    match run_dub(&p.project, &p.script, "en", &p.out, false).await {
        Ok(_) => panic!("an unreachable server must fail, not synthesize"),
        Err(DubError::Validation(v)) => panic!(
            "an unreachable server is a runtime failure (exit 1), not a script \
             problem (exit 2): {v:?}"
        ),
        Err(DubError::Runtime(r)) => {
            assert!(r.contains("127.0.0.1:1"), "must name the url: {r}");
        }
    }
}

// --- Bounded concurrent synthesis. `dub` fans lines out to the backend rather
// than rendering them one at a time; the tests below exercise that against a
// stub that answers both `/v1/audio/voices` (the one-shot check above) and
// `/v1/audio/speech` (actual synthesis).

/// A stub that serves both endpoints `dub` calls against a kokoro backend,
/// and records the peak number of `/v1/audio/speech` requests it had open
/// at once. That peak is the only way to tell fan-out from a serial loop
/// from outside the process: both produce a correct manifest, so a test
/// that only checks the manifest's contents would pass against a serial
/// implementation too and prove nothing about concurrency.
struct KokoroSynthStub {
    base_url: String,
    peak_inflight: Arc<AtomicUsize>,
}

/// PCM bytes for `text`: one i16 sample per character (at least one), so
/// different line texts produce distinguishably different response
/// lengths without a hardcoded lookup table.
fn speech_bytes_for(text: &str) -> Vec<u8> {
    (0..text.chars().count().max(1))
        .flat_map(|i| ((i % 1000) as i16).to_le_bytes())
        .collect()
}

/// [`kokoro_synth_stub`] with the default short delay, for callers that
/// only care about overlap, not about a specific margin.
async fn kokoro_synth_stub(fail_on: Option<&'static str>) -> KokoroSynthStub {
    kokoro_synth_stub_with_delay(fail_on, std::time::Duration::from_millis(40)).await
}

/// Spawns the stub. `fail_on`, when set, makes exactly the `/v1/audio/speech`
/// request whose `input` equals it fail with a 500 — everything else,
/// including the voice list, succeeds. Every successful synthesis is held
/// open for `success_delay` before answering: a stub that replies instantly
/// would read "peak 1 in flight" whether callers dispatched three requests
/// at once or one after another, which would make the concurrency assertion
/// below pass vacuously. A larger `success_delay` also gives a caller enough
/// margin to assert that a *failing* sibling did not wait around for these
/// slower ones to finish.
async fn kokoro_synth_stub_with_delay(
    fail_on: Option<&'static str>,
    success_delay: std::time::Duration,
) -> KokoroSynthStub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let inflight = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let peak_for_stub = peak.clone();
    let voices_body = serde_json::json!({ "voices": ["af_heart"] }).to_string();

    tokio::spawn(async move {
        let peak = peak_for_stub;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let inflight = inflight.clone();
            let peak = peak.clone();
            let voices_body = voices_body.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let raw = String::from_utf8_lossy(&buf[..n]).to_string();

                if raw.starts_with("GET /v1/audio/voices") {
                    write_ok(&mut socket, "application/json", voices_body.as_bytes()).await;
                    return;
                }

                let body_str = raw.split("\r\n\r\n").nth(1).unwrap_or("");
                let body: serde_json::Value =
                    serde_json::from_str(body_str).unwrap_or(serde_json::Value::Null);
                let text = body["input"].as_str().unwrap_or("").to_string();

                if fail_on == Some(text.as_str()) {
                    // Answered immediately, on purpose: this is the request
                    // that is supposed to end the run, and a test asserting
                    // the run does *not* wait on slower siblings needs this
                    // one to be the fast one.
                    write_status(&mut socket, 500, "boom").await;
                    return;
                }

                let now = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(success_delay).await;
                inflight.fetch_sub(1, Ordering::SeqCst);

                let pcm = speech_bytes_for(&text);
                write_ok(&mut socket, "application/octet-stream", &pcm).await;
            });
        }
    });

    KokoroSynthStub {
        base_url: format!("http://{addr}"),
        peak_inflight: peak,
    }
}

async fn write_ok(socket: &mut tokio::net::TcpStream, content_type: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: \
         close\r\n\r\n",
        body.len()
    );
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(body).await;
    let _ = socket.shutdown().await;
}

async fn write_status(socket: &mut tokio::net::TcpStream, code: u16, body: &str) {
    let head = format!(
        "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(body.as_bytes()).await;
    let _ = socket.shutdown().await;
}

/// Three lines against a stub that answers each with a distinguishable
/// length. The manifest must list them in document order regardless of
/// which reply lands first — and the stub's recorded peak in-flight count
/// is the proof that they were actually dispatched concurrently, not that
/// a serial loop happened to produce the right order anyway.
#[tokio::test]
async fn lines_are_synthesized_concurrently_but_collected_in_document_order() {
    let stub = kokoro_synth_stub(None).await;
    let p = project_with_config_and_script(
        &format!(
            "[voice]\nbackend = \"kokoro\"\nvoice = \"af_heart\"\n\n[backends.kokoro]\n\
             base_url = \"{}\"\nconcurrency = 3\n",
            stub.base_url
        ),
        "# Segments\n\nFirst line here.\n\nSecond line here.\n\nThird line here.\n",
    );

    run_dub(&p.project, &p.script, "en", &p.out, false)
        .await
        .unwrap_or_else(|e| match e {
            DubError::Validation(v) => panic!("validation: {v:?}"),
            DubError::Runtime(r) => panic!("runtime: {r}"),
        });

    assert!(
        stub.peak_inflight.load(Ordering::SeqCst) > 1,
        "three lines under concurrency 3 must overlap in flight; a serial \
         render loop would never show more than one request in flight at once"
    );

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path(&p.out, "en")).unwrap())
            .unwrap();
    let ids: Vec<&str> = manifest["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();

    assert_eq!(ids.len(), 3);
    let sorted = {
        let mut c = ids.clone();
        c.sort();
        c
    };
    assert_eq!(ids, sorted, "lines must be in document order: {ids:?}");
}

/// One line's failure fails the run. With fan-out it would be easy to collect
/// every task's result and carry on regardless — a half-dubbed output directory
/// is worse than none. Siblings are given a long (600ms) delay so the test can
/// also pin *how* the run ends: the surfaced error must be the real 500 rather
/// than a cancellation artifact, and the run must return promptly rather than
/// waiting on those siblings to finish first.
#[tokio::test]
async fn a_failure_in_one_line_fails_the_run() {
    let stub =
        kokoro_synth_stub_with_delay(Some("Two."), std::time::Duration::from_millis(600)).await;
    let p = project_with_config_and_script(
        &format!(
            "[voice]\nbackend = \"kokoro\"\nvoice = \"af_heart\"\n\n[backends.kokoro]\n\
             base_url = \"{}\"\nconcurrency = 3\n",
            stub.base_url
        ),
        "# Failure\n\nOne.\n\nTwo.\n\nThree.\n",
    );
    let start = std::time::Instant::now();
    let result = run_dub(&p.project, &p.script, "en", &p.out, false).await;
    let elapsed = start.elapsed();
    match result {
        Ok(_) => panic!("a line failure must fail the run"),
        Err(DubError::Runtime(r)) => {
            assert!(
                // Not a bare "500": the message also embeds the stub's
                // ephemeral port, and a small fraction of ports contain the
                // digits "500" too — that would make this assertion pass
                // for the wrong reason on an unlucky port.
                r.contains("returned 500"),
                "the surfaced error must be the real \
                failure, not a cancellation artifact of aborting the \
                siblings: {r}"
            );
        }
        Err(DubError::Validation(v)) => panic!(
            "a synthesis failure is a runtime error, not a validation one: {}",
            v.join("; ")
        ),
    }
    assert!(!manifest_path(&p.out, "en").exists(), "no partial output");
    assert!(
        elapsed < std::time::Duration::from_millis(300),
        "the run must return once the failure is known, not wait on the \
         600ms siblings still in flight: took {elapsed:?}"
    );
}

// --- `teleprompt_cache::key` hashes backend id/version, locale, voice,
// speed, and the *text*, not the line id — so two lines with
// identical narration text collide on one `CacheKey` by design. Rendering
// each occurrence independently would double-count synthesis work against
// the exact bottleneck fan-out exists to relieve, and two concurrent
// `cache.store` calls under the same key would race on `std::fs::write`'s
// truncate-then-write. The tests below pin that `dub` renders each distinct
// key once and fans the result out to every line that shares it.

/// Answers `/v1/audio/voices` with `["af_heart"]` and `/v1/audio/speech`
/// with PCM whose length depends on which call this is — the first
/// `/v1/audio/speech` request gets a different length than the second. A
/// real TTS server is not bit-deterministic between calls, so a caller that
/// (incorrectly) issues two synthesis requests for identical text would get
/// back two different lengths for what must be one duration in the
/// manifest; a caller that (correctly) issues one request has no second
/// call to differ from. Also records the total number of `/v1/audio/speech`
/// requests received, so a test can assert on it directly rather than
/// inferring de-duplication from a side effect.
struct KokoroCallCountingStub {
    base_url: String,
    speech_requests: Arc<AtomicUsize>,
}

async fn kokoro_call_counting_stub() -> KokoroCallCountingStub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let speech_requests = Arc::new(AtomicUsize::new(0));
    let speech_requests_for_stub = speech_requests.clone();
    let voices_body = serde_json::json!({ "voices": ["af_heart"] }).to_string();

    tokio::spawn(async move {
        let speech_requests = speech_requests_for_stub;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let speech_requests = speech_requests.clone();
            let voices_body = voices_body.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let raw = String::from_utf8_lossy(&buf[..n]).to_string();

                if raw.starts_with("GET /v1/audio/voices") {
                    write_ok(&mut socket, "application/json", voices_body.as_bytes()).await;
                    return;
                }

                let call = speech_requests.fetch_add(1, Ordering::SeqCst);
                // 2400 i16 samples = 100ms at 24kHz; each successive call
                // to this stub gets 100ms more than the last, so call 0 and
                // call 1 are observably different lengths.
                let samples = 2400 * (call + 1);
                let pcm: Vec<u8> = (0..samples)
                    .flat_map(|i| ((i % 1000) as i16).to_le_bytes())
                    .collect();
                write_ok(&mut socket, "application/octet-stream", &pcm).await;
            });
        }
    });

    KokoroCallCountingStub {
        base_url: format!("http://{addr}"),
        speech_requests,
    }
}

/// A project whose config points at `stub` with `concurrency = 8`, and a
/// script with two lines sharing the exact same narration text — the
/// reproduction the finding used. `concurrency` is set well above the
/// line count so a version that (incorrectly) renders per-occurrence
/// rather than per-key has every opportunity to fan the duplicate work out
/// concurrently, rather than happening to serialize it back into one
/// request by accident.
fn project_with_duplicate_narration_text(stub: &str) -> TestProject {
    project_with_config_and_script(
        &format!(
            "[voice]\nbackend = \"kokoro\"\nvoice = \"af_heart\"\n\n[backends.kokoro]\n\
             base_url = \"{stub}\"\nconcurrency = 8\n"
        ),
        "# Dup\n\nThe very same sentence.\n\nThe very same sentence.\n",
    )
}

#[tokio::test]
async fn identical_narration_text_synthesizes_once_not_once_per_line() {
    let stub = kokoro_call_counting_stub().await;
    let p = project_with_duplicate_narration_text(&stub.base_url);

    run_dub(&p.project, &p.script, "en", &p.out, false)
        .await
        .unwrap_or_else(|e| match e {
            DubError::Validation(v) => panic!("validation: {v:?}"),
            DubError::Runtime(r) => panic!("runtime: {r}"),
        });

    assert_eq!(
        stub.speech_requests.load(Ordering::SeqCst),
        1,
        "two lines with identical text share one cache key; rendering \
         each occurrence independently duplicates work against the exact \
         bottleneck fan-out exists to relieve"
    );
}

/// The stub's two calls would answer with different-length audio; if `dub`
/// issued one synthesis request per occurrence, one of the two lines
/// would end up with a `rendered_ms` that disagrees with what the
/// recompiled timeline publishes for it (both lines resolve to the same
/// cache key, so the timeline can only publish one duration), and the
/// length-mismatch guard would fail the run. Rendering by key rather than
/// by occurrence means there is only ever one real answer to disagree with
/// itself.
#[tokio::test]
async fn identical_narration_text_with_a_non_deterministic_backend_still_succeeds() {
    let stub = kokoro_call_counting_stub().await;
    let p = project_with_duplicate_narration_text(&stub.base_url);

    run_dub(&p.project, &p.script, "en", &p.out, false)
        .await
        .unwrap_or_else(|e| match e {
            DubError::Validation(v) => panic!("validation: {v:?}"),
            DubError::Runtime(r) => panic!("runtime: {r}"),
        });

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path(&p.out, "en")).unwrap())
            .unwrap();
    let segs = manifest["lines"].as_array().unwrap();
    assert_eq!(segs.len(), 2);
    let durations: Vec<u64> = segs
        .iter()
        .map(|s| s["duration_ms"].as_u64().unwrap())
        .collect();
    assert_eq!(
        durations[0], durations[1],
        "both lines resolve to one cache key and must publish the same \
         duration: {durations:?}"
    );
}

/// The other half of the finding: fanning identical-text lines into one
/// task must not lose track of *which* line is which. Both must still
/// land in their own document-ordered slot, and — since they share one
/// cache key — both must carry the exact same audio bytes.
#[tokio::test]
async fn identical_narration_text_still_lands_in_document_order_with_matching_audio() {
    let stub = kokoro_call_counting_stub().await;
    let p = project_with_duplicate_narration_text(&stub.base_url);

    run_dub(&p.project, &p.script, "en", &p.out, false)
        .await
        .unwrap_or_else(|e| match e {
            DubError::Validation(v) => panic!("validation: {v:?}"),
            DubError::Runtime(r) => panic!("runtime: {r}"),
        });

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path(&p.out, "en")).unwrap())
            .unwrap();
    let segs = manifest["lines"].as_array().unwrap();
    assert_eq!(segs.len(), 2);

    let ids: Vec<&str> = segs.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_ne!(ids[0], ids[1], "two distinct lines sharing one render");
    let sorted = {
        let mut c = ids.clone();
        c.sort();
        c
    };
    assert_eq!(
        ids, sorted,
        "document order must hold even when lines share a render: {ids:?}"
    );

    let bytes: Vec<Vec<u8>> = segs.iter().map(|s| wav_bytes(&p.project.root, s)).collect();
    assert_eq!(
        bytes[0], bytes[1],
        "both lines resolve to the same cache key, so both must carry \
         the same audio bytes"
    );
}

/// A cache that is already warm should not need the server that filled it.
///
/// `dub` asks the server for its voice list before synthesizing, which is
/// the right check when something is about to be synthesized and the wrong
/// one when nothing is: a project whose every line is cached failed
/// outright with the server down. That is the difference between "build it
/// once and let the cache make the next one cheap" being a workflow and
/// being something you can only do while a GPU box answers.
#[tokio::test]
async fn a_fully_cached_script_dubs_with_the_server_gone() {
    let stub = kokoro_synth_stub(None).await;
    let p = project_with_config_and_script(
        &format!(
            "[voice]\nbackend = \"kokoro\"\nvoice = \"af_heart\"\n\n[backends.kokoro]\n\
             base_url = \"{}\"\n",
            stub.base_url
        ),
        "# Warm\n\nOne paragraph, synthesized once. {#one}\n",
    );

    // Fill the cache while the server is up.
    run_dub(&p.project, &p.script, "en", &p.out, false)
        .await
        .unwrap_or_else(|e| match e {
            DubError::Validation(v) => panic!("validation: {v:?}"),
            DubError::Runtime(r) => panic!("runtime: {r}"),
        });

    // The same project on the same machine the next morning: the cache is
    // where it was, the server is not. Nothing is missing, so nothing
    // should need it.
    std::fs::write(
        p.project.root.join("teleprompt.toml"),
        "[voice]\nbackend = \"kokoro\"\nvoice = \"af_heart\"\n\n[backends.kokoro]\n\
         base_url = \"http://127.0.0.1:9\"\n",
    )
    .unwrap();
    let gone = Project::discover(&p.project.root).unwrap();

    run_dub(&gone, &p.script, "en", &p.out, false)
        .await
        .unwrap_or_else(|e| match e {
            DubError::Validation(v) => panic!("validation: {v:?}"),
            DubError::Runtime(r) => panic!("a warm cache must not need the server: {r}"),
        });
}

const TOUR: &str = "\
# Tour

Welcome to Acme. {#welcome}

Deployment is one command. {#deploy}
";

/// A project with `welcome` recorded, at another rate than the voice's.
fn recorded(tag: &str) -> teleprompt_testkit::TestDir {
    let root = project_with(tag, TOUR);
    let mut takes = teleprompt_voice::takes::Takes::load(&root.join("takes")).unwrap();
    let pcm = teleprompt_voice::Pcm {
        sample_rate: 24_000,
        channels: 1,
        samples: (0..36_000).map(|i| ((i % 60) * 300) as i16).collect(),
    };
    takes.save("welcome", "Welcome to Acme.", &pcm).unwrap();
    root
}

/// A recorded line is published from its take, at the rate the rest of the
/// narration has and at exactly its own length; the others are synthesized,
/// and dub says which.
#[test]
fn dub_publishes_a_take_and_names_the_lines_it_synthesized() {
    let root = recorded("take");
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(code(&out), 0, "{stderr}");

    let m = read_manifest(&root);
    let lines = m["lines"].as_array().unwrap();
    let welcome = lines.iter().find(|l| l["id"] == "welcome").unwrap();
    assert_eq!(welcome["duration_ms"], 1500);
    assert_eq!(wav_ms(&root, &m, welcome), 1500);
    let wav = teleprompt_voice::wav::decode(&wav_bytes(&root, welcome)).unwrap();
    assert_eq!(
        u64::from(wav.sample_rate),
        m["audio"]["sample_rate"].as_u64().unwrap()
    );
    assert!(
        wav.samples.iter().any(|&s| s != 0),
        "the take's sound, not silence"
    );

    assert!(
        stderr.contains("synthesized") && stderr.contains("deploy") && !stderr.contains("welcome"),
        "{stderr}"
    );
}

/// A take whose audio is not what was recorded is refused, not published.
#[test]
fn dub_refuses_a_take_that_changed_on_disk() {
    let root = recorded("take-tampered");
    std::fs::write(root.join("takes/welcome.wav"), b"RIFF not the take").unwrap();
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(code(&out), 1, "{stderr}");
    assert!(stderr.contains("welcome.wav"), "{stderr}");
}

/// A `fit-line` line is written at the tempo that fits its picture: its
/// audio is as long as the manifest says, and the manifest says the tempo
/// (docs/design.md#led-by-the-picture).
#[test]
fn dub_writes_a_fit_line_line_at_its_tempo() {
    let script = "---\nteleprompt: 1\n---\n\n# Fit\n\n\
        Deployment is one command, and it streams progress as it goes. {#deploy}\n\n\
        ```teleprompt scene=mock policy=fit-line\nwait 1000ms\n```\n";
    let root = project_with("fit-line", script);
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let m = read_manifest(&root);
    let line = &m["lines"][0];
    assert_eq!(line["tempo_permille"], 1150, "{line}");
    let wav = std::fs::read(
        root.join("public/narration/en")
            .join(line["audio"].as_str().unwrap()),
    )
    .unwrap();
    let pcm = teleprompt_voice::wav::decode(&wav).unwrap();
    assert_eq!(pcm.duration_ms(), line["duration_ms"].as_u64().unwrap());
}
