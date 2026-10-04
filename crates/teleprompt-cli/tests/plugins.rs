//! Plugins installed as programs of their own: found, listed with what they
//! need, and compiled against as a built-in scene plugin is. Uses the example
//! plugins in `examples/plugins`, which are Python.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins")
}

fn have_python() -> bool {
    let present = Command::new("python3")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok();
    if !present {
        eprintln!("skipped: no python3 to run the example plugins");
    }
    present
}

/// `teleprompt args…` in `dir`, finding plugins in `plugins` and the
/// examples, and nowhere else of the machine's.
fn teleprompt(dir: &std::path::Path, plugins: &std::path::Path, args: &[&str]) -> Output {
    let path = std::env::join_paths([examples()].into_iter().chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(dir)
        .env("PATH", path)
        .env("TELEPROMPT_PLUGINS", plugins)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn installed_plugins_are_listed_with_what_they_need() {
    if !have_python() {
        return;
    }
    let dir = teleprompt_testkit::test_dir("plugins-list");
    // A plugin a built-in scene plugin's name hides, in the plugins directory.
    let mock = dir.join("teleprompt-scene-mock");
    std::fs::write(&mock, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&mock, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let out = teleprompt(&dir, &dir, &["--format", "json", "plugins"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let plugins = report["plugins"].as_array().unwrap();
    let named = |name: &str| plugins.iter().find(|p| p["name"] == name).unwrap().clone();
    let card = named("card");
    assert_eq!(card["kind"], "scene");
    assert_eq!(card["needs"], serde_json::json!(["ffmpeg"]));
    assert!(card["problem"].is_null(), "{card}");
    assert_eq!(named("espeak")["kind"], "voice");
    let hidden = named("mock");
    assert!(
        hidden["problem"].as_str().unwrap().contains("built-in"),
        "{hidden}"
    );
}

#[test]
fn a_script_compiles_against_an_installed_scene_plugin() {
    if !have_python() {
        return;
    }
    let dir = teleprompt_testkit::test_dir("plugins-check");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/cards.md");
    std::fs::write(
        &script,
        "---\nteleprompt: 1\n---\n\n# Cards\n\nA blue card. {#blue}\n\n```teleprompt scene=card\ncolor #1d4ed8\nhold 2s\n```\n",
    )
    .unwrap();
    let out = teleprompt(
        &dir,
        &dir,
        &["--format", "json", "plan", "scripts/cards.md"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let plan = String::from_utf8_lossy(&out.stdout);
    assert!(plan.contains("blue-a#0"), "{plan}");

    // A bad line is reported where it is, in the plugin's own words.
    std::fs::write(
        &script,
        "---\nteleprompt: 1\n---\n\n# Cards\n\nA blue card. {#blue}\n\n```teleprompt scene=card\ncolour blue\n```\n",
    )
    .unwrap();
    let out = teleprompt(&dir, &dir, &["check", "scripts/cards.md"]);
    let said = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{said}");
    assert!(said.contains("unknown card directive `colour`"), "{said}");
    assert!(said.contains("cards.md:10"), "{said}");
}
