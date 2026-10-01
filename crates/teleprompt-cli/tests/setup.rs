//! `teleprompt setup`: the tools adapters run, detected, and installed with
//! the author's own package manager, never shipped
//! (docs/design.md#what-teleprompt-ships).

use std::path::Path;
use std::process::{Command, Output};

use teleprompt_cli::cmd::setup::{resolve, tools, Manager, Platform};

fn mac() -> Platform {
    Platform::new("macos", &[Manager::Brew])
}

fn ubuntu() -> Platform {
    Platform::new("linux", &[Manager::Apt])
}

fn command(name: &str, platform: &Platform) -> Option<String> {
    resolve(&[name.to_string()]).unwrap()[0].command(platform)
}

fn names(given: &[&str]) -> Vec<&'static str> {
    let given: Vec<String> = given.iter().map(|s| s.to_string()).collect();
    resolve(&given).unwrap().iter().map(|t| t.name).collect()
}

#[test]
fn a_tool_installs_with_the_platforms_own_manager() {
    assert_eq!(
        command("ffmpeg", &mac()).as_deref(),
        Some("brew install ffmpeg")
    );
    assert_eq!(
        command("ffmpeg", &ubuntu()).as_deref(),
        Some("sudo apt-get install -y ffmpeg")
    );
}

/// A distribution that does not package it: its language's own installer.
#[test]
fn a_tool_without_a_package_falls_back_to_its_languages_installer() {
    let with_go = Platform::new("linux", &[Manager::Apt, Manager::Go]);
    assert_eq!(
        command("vhs", &with_go).as_deref(),
        Some("go install github.com/charmbracelet/vhs@latest")
    );
    assert_eq!(command("vhs", &ubuntu()), None);
}

/// What each adapter says it runs, capturing and then recording: the
/// tools `doctor` sends an author to `setup <adapter>` for.
#[test]
fn an_adapter_names_the_tools_it_runs() {
    assert_eq!(names(&["vhs"]), ["vhs", "ttyd", "ffmpeg"]);
    assert_eq!(names(&["asciinema"]), ["agg", "ffmpeg", "asciinema"]);
    assert_eq!(names(&["playwright"]), ["node", "ffmpeg", "playwright"]);
    assert_eq!(names(&["remotion"]), ["node", "remotion"]);
    assert_eq!(names(&["slidev"]), ["node", "ffmpeg", "slidev"]);
    assert_eq!(names(&["media"]), ["ffmpeg"]);
    assert_eq!(names(&["mock"]), ["ffmpeg"]);
    assert_eq!(names(&["prompt"]), ["speech-model"]);
}

/// Every adapter this build has, down to one only for tests, names only
/// tools `setup` knows.
#[test]
fn every_adapter_needs_only_tools_setup_knows() {
    for adapter in teleprompt_cli::scene::captures().backends() {
        let needs = teleprompt_cli::scene::needs(adapter.adapter()).unwrap();
        assert!(!needs.is_empty(), "{}", adapter.adapter());
        resolve(&[adapter.adapter().to_string()]).unwrap();
    }
}

#[test]
fn a_tool_named_twice_is_listed_once() {
    assert_eq!(
        names(&["vhs", "media", "ffmpeg"]),
        ["vhs", "ttyd", "ffmpeg"]
    );
}

#[test]
fn an_unknown_name_says_what_there_is() {
    let err = resolve(&["obs".to_string()]).unwrap_err();
    assert!(
        err.contains("obs") && err.contains("ffmpeg") && err.contains("vhs"),
        "{err}"
    );
}

#[test]
fn every_tool_says_its_license_and_where_it_is_from() {
    for tool in tools() {
        assert!(!tool.license.is_empty(), "{}", tool.name);
        assert!(tool.home.starts_with("https://"), "{}", tool.name);
    }
}

/// A tool that lives in the author's own project, or is a server they run,
/// is not installed for them: `setup` says what to do instead.
#[test]
fn a_tool_of_the_authors_own_is_explained_not_installed() {
    let remotion = &resolve(&["remotion".to_string()]).unwrap()[1];
    assert_eq!(remotion.name, "remotion");
    assert_eq!(remotion.command(&mac()), None);
    assert!(remotion.guide.unwrap().contains("npm install"));
    assert!(remotion.license.contains("company license"));
}

/// A directory standing in for PATH, with the programs a test gives it.
struct Bin(teleprompt_testkit::TestDir);

impl Bin {
    fn new(tag: &str) -> Self {
        Self(teleprompt_testkit::test_dir(tag))
    }

    fn script(&self, name: &str, body: &str) {
        let path = self.0.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
    }

    fn setup(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_teleprompt"))
            .arg("setup")
            .args(args)
            .current_dir(self.0.path())
            .env("PATH", self.0.path())
            .output()
            .unwrap()
    }
}

/// A `brew` that installs a working `name` into the same directory.
fn brew_installing(bin: &Bin, log: &Path) {
    bin.script(
        "brew",
        &format!(
            "echo \"$@\" >> {log}\nprintf '#!/bin/sh\\n' > {dir}/$2\n/bin/chmod +x {dir}/$2",
            log = log.display(),
            dir = bin.0.path().display()
        ),
    );
}

#[test]
fn setup_says_what_is_missing_its_license_and_the_command() {
    let bin = Bin::new("setup-list");
    bin.script("brew", "exit 0");
    let out = bin.setup(&["media", "--format", "json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ffmpeg = &report["tools"][0];
    assert_eq!(ffmpeg["name"], "ffmpeg");
    assert_eq!(ffmpeg["installed"], false);
    assert_eq!(ffmpeg["command"], "brew install ffmpeg");
    assert!(
        ffmpeg["license"].as_str().unwrap().contains("LGPL"),
        "{ffmpeg}"
    );
    // Nothing was run.
    assert!(!bin.0.join("ffmpeg").exists());

    let human = String::from_utf8_lossy(&bin.setup(&["media"]).stdout).to_string();
    assert!(
        human.contains("brew install ffmpeg") && human.contains("--run"),
        "{human}"
    );
}

#[test]
fn setup_run_installs_what_is_missing() {
    let bin = Bin::new("setup-run");
    let log = bin.0.join("brew.log");
    brew_installing(&bin, &log);
    let out = bin.setup(&["media", "--run"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "install ffmpeg\n");
    assert!(String::from_utf8_lossy(&out.stdout).contains("ffmpeg"));

    // Installed now: nothing to run again.
    let again = bin.setup(&["media", "--run"]);
    assert!(again.status.success());
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "install ffmpeg\n");
}

#[test]
fn setup_run_that_fails_says_which_command() {
    let bin = Bin::new("setup-fail");
    bin.script("brew", "exit 3");
    let out = bin.setup(&["media", "--run"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("brew install ffmpeg"), "{err}");
}

#[test]
fn setup_with_no_way_to_install_says_where_the_tool_is_from() {
    let bin = Bin::new("setup-none");
    let out = bin.setup(&["vhs", "--format", "json"]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["tools"][0]["name"], "vhs");
    assert!(report["tools"][0]["command"].is_null());
    assert_eq!(
        report["tools"][0]["home"],
        "https://github.com/charmbracelet/vhs"
    );
}

/// Voicebox is an app of the author's; its models' licenses differ, and
/// one's weights come with conditions.
#[test]
fn voicebox_is_explained_with_its_models_licenses() {
    let voicebox = &resolve(&["voicebox".to_string()]).unwrap()[0];
    assert_eq!(voicebox.command(&mac()), None);
    assert!(voicebox.guide.unwrap().contains("teleprompt voice clone"));
    assert!(
        voicebox.license.contains("MIT") && voicebox.license.contains("Llama"),
        "{}",
        voicebox.license
    );
}

/// Gemini is a service, not a program: `setup` says where the key comes
/// from and that the narration goes to Google, and installs nothing.
#[test]
fn gemini_is_explained_as_a_service() {
    let gemini = &resolve(&["gemini".to_string()]).unwrap()[0];
    assert_eq!(gemini.command(&mac()), None);
    let guide = gemini.guide.unwrap();
    assert!(
        guide.contains("GEMINI_API_KEY") && guide.contains("Google"),
        "{guide}"
    );
}

/// Each use `setup` asks about names what it needs, and is a name `setup`
/// takes too: `setup conversations` is the models a recording needs.
#[test]
fn every_goal_is_a_name_setup_takes() {
    use teleprompt_cli::cmd::setup::GOALS;
    for goal in GOALS {
        let tools =
            resolve(&[goal.name.to_string()]).unwrap_or_else(|e| panic!("{}: {e}", goal.name));
        assert!(!tools.is_empty(), "{}", goal.name);
        // A goal is not also a tool's name, which it would hide.
        assert!(
            teleprompt_cli::cmd::setup::tools()
                .iter()
                .all(|t| t.name != goal.name),
            "{}",
            goal.name
        );
    }
    assert_eq!(
        names(&["conversations"]),
        ["speech-model", "punctuation-model", "speaker-model"]
    );
    assert_eq!(names(&["prompt"]), ["speech-model"]);
}

/// Only what listens needs the build that can: the rest is offered in
/// every build.
#[test]
fn only_listening_goals_need_the_speech_models() {
    use teleprompt_cli::cmd::setup::{download_mb, GOALS};
    for goal in GOALS {
        let tools = resolve(&[goal.name.to_string()]).unwrap();
        let models = tools.iter().any(|t| download_mb(t.name).is_some());
        assert_eq!(goal.listens, models, "{}", goal.name);
    }
}
