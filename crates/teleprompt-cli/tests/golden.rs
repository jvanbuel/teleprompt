//! Golden output: what every project in the repository compiles to, and what
//! every command prints and exits with when it fails.
//!
//! These snapshots pin behaviour while the code under them is restructured.
//! A refactor leaves them untouched; a snapshot that moves is a behaviour
//! change, and belongs in a commit that says so.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

/// Each project, and the script in it, by path from the repository root.
const PROJECTS: &[&str] = &[
    "manual/scripts/cli.md",
    "demos/flowrs/scripts/demo.md",
    "examples/asciinema/scripts/recording.md",
    "examples/media/scripts/tour.md",
    "examples/remotion/scripts/remotion.md",
    "examples/slidev/scripts/slides.md",
];

/// Projects whose configured voice needs nothing installed, so `dub` runs.
const DUBBABLE: &[&str] = &["manual/scripts/cli.md", "demos/flowrs/scripts/demo.md"];

/// Caches, outputs and installs: whatever a local run leaves behind that a
/// fresh checkout does not have.
const NOT_SOURCES: &[&str] = &[".teleprompt", "build", "node_modules", "public", "target"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Copies the project `script` belongs to into `dest`, at the same path from
/// the root: configs name their assets relative to the repository root.
fn copy_project(script: &str, dest: &Path) {
    let root = repo_root();
    let mut dir = root.join(script).parent().unwrap().to_path_buf();
    while !dir.join("teleprompt.toml").exists() {
        dir = dir.parent().unwrap().to_path_buf();
    }
    copy_dir(&dir, &dest.join(dir.strip_prefix(&root).unwrap()));
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if NOT_SOURCES.iter().any(|n| name == *n) {
            continue;
        }
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&path, &to.join(&name));
        } else {
            std::fs::copy(&path, to.join(&name)).unwrap();
        }
    }
}

/// Runs the binary in `cwd`, killing it if it has not exited in 30 seconds.
fn tp(cwd: &Path, args: &[&str]) -> Output {
    let child = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Read while it runs, on a thread: a child that fills a pipe nobody
    // reads blocks on it, and would look like one that never exits.
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(out) => out.unwrap(),
        Err(_) => {
            let _ = Command::new("kill").arg(pid.to_string()).status();
            panic!("`teleprompt {}` did not exit", args.join(" "));
        }
    }
}

/// Exit code, stdout (as JSON where it is JSON) and stderr, with the one
/// thing that differs from run to run — a listening port — masked.
fn transcript(out: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stderr: Vec<String> = stderr.lines().map(mask_port).collect();
    json!({
        "exit": out.status.code(),
        "stdout": serde_json::from_str::<Value>(&stdout).unwrap_or_else(|_| json!(stdout)),
        "stderr": stderr,
    })
}

fn mask_port(line: &str) -> String {
    match line.find("http://127.0.0.1:") {
        Some(at) => {
            let start = at + "http://127.0.0.1:".len();
            let end = line[start..]
                .find(|c: char| !c.is_ascii_digit())
                .map_or(line.len(), |n| start + n);
            format!("{}PORT{}", &line[..start], &line[end..])
        }
        None => line.to_string(),
    }
}

/// The project's directory name: `manual`, `flowrs`, `media`, …
fn slug(script: &str) -> &str {
    let project = script.split("/scripts/").next().unwrap();
    project.rsplit('/').next().unwrap()
}

#[test]
fn every_project_plans_and_checks_as_before() {
    for script in PROJECTS {
        let dir = tempfile::tempdir().unwrap();
        copy_project(script, dir.path());
        for cmd in ["plan", "check"] {
            let out = tp(dir.path(), &[cmd, script, "--format", "json"]);
            insta::assert_json_snapshot!(format!("{cmd}-{}", slug(script)), transcript(&out));
        }
    }
}

#[test]
fn every_offline_project_dubs_as_before() {
    for script in DUBBABLE {
        let dir = tempfile::tempdir().unwrap();
        copy_project(script, dir.path());
        let out = tp(
            dir.path(),
            &["dub", script, "--out", "narration", "--format", "json"],
        );
        let manifest = std::fs::read_to_string(dir.path().join("narration/en/narration.json"))
            .map(|raw| serde_json::from_str::<Value>(&raw).unwrap())
            .unwrap_or(Value::Null);
        insta::assert_json_snapshot!(
            format!("dub-{}", slug(script)),
            json!({ "run": transcript(&out), "manifest": manifest })
        );
    }
}

#[test]
fn every_command_fails_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    teleprompt_cli::cmd::new::scaffold(&root.join("proj")).unwrap();
    std::fs::write(
        root.join("proj/scripts/bad.md"),
        "Before any heading.\n\n```teleprompt\nnope\n```\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("loose")).unwrap();
    std::fs::write(root.join("loose/orphan.md"), "# Orphan\n\nNo project.\n").unwrap();

    let scripts = [
        ("missing", "proj/scripts/missing.md"),
        ("invalid", "proj/scripts/bad.md"),
        ("orphan", "loose/orphan.md"),
    ];
    let commands: &[(&str, &[&str])] = &[
        ("check", &["check"]),
        ("plan", &["plan"]),
        ("plan-check", &["plan", "--check"]),
        ("dub", &["dub", "--out", "out"]),
        ("build", &["build"]),
        ("capture", &["capture"]),
        ("serve-voice", &["serve", "--voice", "--port", "0"]),
    ];
    for (label, cmd) in commands {
        let mut runs = serde_json::Map::new();
        for (case, script) in scripts {
            for format in ["human", "json"] {
                let mut args = cmd.to_vec();
                args.extend([script, "--format", format]);
                runs.insert(format!("{case} {format}"), transcript(&tp(root, &args)));
            }
        }
        insta::assert_json_snapshot!(format!("fail-{label}"), runs);
    }

    let mut runs = serde_json::Map::new();
    for (case, args) in [
        ("new existing", &["new", "proj"][..]),
        ("import missing", &["import", "proj/nothing.md"][..]),
    ] {
        for format in ["human", "json"] {
            let mut args = args.to_vec();
            args.extend(["--format", format]);
            runs.insert(format!("{case} {format}"), transcript(&tp(root, &args)));
        }
    }
    insta::assert_json_snapshot!("fail-new-import", runs);
}
