//! `teleprompt translate`: fills in a locale's translation, and only what
//! is missing or out of date.

use std::path::Path;
use std::process::{Command, Output};

const SCRIPT: &str = "\
# Introduction

Welcome to Acme. {#welcome}

Run flowrs config add to start. {#start}

```teleprompt scene=mock policy=concurrent cue=\"config add\"
wait 300ms
```
";

/// A translator that prefixes each item with its locale, keeps cues as
/// they are, and logs the ids it was asked for.
const FAKE: &str = r#"python3 -c '
import json, sys
r = json.load(sys.stdin)
open("asked.log", "a").write(" ".join(i["id"] for i in r["translate"]) + "\n")
out = [{"id": i["id"], "text": i["english"] if i["kind"] == "cue" else "[" + r["target"] + "] " + i["english"]} for i in r["translate"]]
print(json.dumps({"items": out}))
'"#;

fn project(tag: &str) -> teleprompt_testkit::TestDir {
    let dir = teleprompt_testkit::test_dir(&format!("translate-{tag}"));
    teleprompt_project::new::scaffold(&dir).unwrap();
    std::fs::write(dir.join("scripts/tour.md"), SCRIPT).unwrap();
    dir
}

fn tp(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn translate(root: &Path) -> String {
    let out = tp(
        root,
        &[
            "translate",
            "scripts/tour.md",
            "--to",
            "nl",
            "--command",
            FAKE,
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn asked(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("asked.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn a_new_locale_is_translated_and_then_compiles() {
    let dir = project("new");
    let report = translate(&dir);
    assert!(report.contains("scripts/tour.nl.yaml"), "{report}");
    let yaml = std::fs::read_to_string(dir.join("scripts/tour.nl.yaml")).unwrap();
    assert!(yaml.contains("[nl] Welcome to Acme."), "{yaml}");
    assert!(yaml.contains("start-a:"), "the cue is kept: {yaml}");
    let plan = tp(&dir, &["plan", "scripts/tour.md", "--locale", "nl"]);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    assert_eq!(
        asked(&dir),
        ["chapter:introduction line:welcome line:start cue:start-a"]
    );
}

#[test]
fn only_what_changed_is_translated_again() {
    let dir = project("again");
    translate(&dir);
    let before = std::fs::read_to_string(dir.join("scripts/tour.nl.yaml")).unwrap();
    let report = translate(&dir);
    assert!(report.contains("up to date"), "{report}");
    assert_eq!(
        std::fs::read_to_string(dir.join("scripts/tour.nl.yaml")).unwrap(),
        before
    );

    let edited = SCRIPT.replace("Welcome to Acme.", "Welcome to Acme Cloud.");
    std::fs::write(dir.join("scripts/tour.md"), edited).unwrap();
    translate(&dir);
    assert_eq!(asked(&dir).last().unwrap(), "line:welcome");
    let yaml = std::fs::read_to_string(dir.join("scripts/tour.nl.yaml")).unwrap();
    assert!(yaml.contains("[nl] Welcome to Acme Cloud."), "{yaml}");
    assert!(yaml.contains("[nl] Run flowrs"), "the rest is kept: {yaml}");
}

#[test]
fn the_source_locale_is_not_a_target() {
    let dir = project("source");
    let out = tp(
        &dir,
        &[
            "translate",
            "scripts/tour.md",
            "--to",
            "en",
            "--command",
            "true",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The provider comes from teleprompt.toml, with its own settings under
/// [translate.<provider>].
#[test]
fn the_provider_is_configured_in_the_project() {
    let dir = project("configured");
    let toml = std::fs::read_to_string(dir.join("teleprompt.toml")).unwrap();
    std::fs::write(
        dir.join("teleprompt.toml"),
        format!("{toml}\n[translate]\nprovider = \"command\"\n\n[translate.command]\nrun = '''{FAKE}'''\n"),
    )
    .unwrap();
    let out = tp(&dir, &["translate", "scripts/tour.md", "--to", "nl"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::read_to_string(dir.join("scripts/tour.nl.yaml"))
        .unwrap()
        .contains("[nl] Welcome"));
}

/// A translator's settings are not a voice's: the project still compiles
/// with them, and with a voice of the same name configured beside them.
#[test]
fn a_translator_setting_is_not_a_voice() {
    let dir = project("apart");
    let toml = std::fs::read_to_string(dir.join("teleprompt.toml")).unwrap();
    std::fs::write(
        dir.join("teleprompt.toml"),
        format!(
            "{toml}\n[translate.ollama]\nurl = \"http://gpu-box:11434\"\n\n\
             [translate.openai]\nurl = \"http://localhost:1234/v1\"\n"
        ),
    )
    .unwrap();
    let out = tp(&dir, &["check", "scripts/tour.md"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// By default a local model translates; with none running, the error says
/// how to get one.
#[test]
fn the_default_is_a_local_model() {
    let dir = project("default");
    let out = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(&dir)
        .env("OLLAMA_HOST", "127.0.0.1:9")
        .args(["translate", "scripts/tour.md", "--to", "nl"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("Ollama") && err.contains("ollama pull gemma3:12b"),
        "{err}"
    );
}
