use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

#[test]
fn the_wav_length_matches_the_duration_the_manifest_claims() {
    let root = project_with("length", SCRIPT);
    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );

    let m = read_manifest(&root);
    let seg = &m["segments"][0];
    let claimed_ms = seg["duration_ms"].as_u64().unwrap();

    let bytes = std::fs::read(
        root.join("public/narration/en")
            .join(seg["audio"].as_str().unwrap()),
    )
    .unwrap();
    let data_len = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as u64;
    let actual_ms = data_len / 2 * 1000 / 48_000;

    assert_eq!(
        actual_ms, claimed_ms,
        "a consumer placing this file at its stated duration must not clip it"
    );
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
