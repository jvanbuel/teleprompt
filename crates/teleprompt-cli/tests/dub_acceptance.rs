use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use teleprompt_cli::project::Project;

fn tempdir(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "teleprompt-dub-acceptance-{tag}-{}-{:?}",
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

const TOUR: &str = include_str!("../../../tests/fixtures/tour.md");

#[test]
fn dub_produces_the_document_a_consumer_will_read() {
    let root = project_with("acceptance", TOUR);
    let out = tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    let raw = std::fs::read_to_string(root.join("public/narration/en/narration.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&raw).unwrap();
    insta::assert_json_snapshot!(manifest);
}

#[test]
fn the_committed_manifest_gates_ci_the_way_the_timeline_does() {
    let root = project_with("gate", TOUR);
    tp(
        &root,
        &["dub", "scripts/test.md", "--out", "public/narration"],
    );

    assert_eq!(
        code(&tp(
            &root,
            &[
                "dub",
                "scripts/test.md",
                "--out",
                "public/narration",
                "--check"
            ]
        )),
        0,
        "clean tree passes"
    );

    std::fs::write(
        root.join("scripts/test.md"),
        format!("{TOUR}\nOne more sentence, added late.\n"),
    )
    .unwrap();

    assert_eq!(
        code(&tp(
            &root,
            &[
                "dub",
                "scripts/test.md",
                "--out",
                "public/narration",
                "--check"
            ]
        )),
        3,
        "an edit that was never re-dubbed must fail the build"
    );
}
