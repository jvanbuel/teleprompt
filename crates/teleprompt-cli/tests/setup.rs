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

#[test]
fn an_adapter_names_the_tools_it_runs() {
    assert_eq!(names(&["vhs"]), ["vhs", "ttyd", "ffmpeg"]);
    assert_eq!(names(&["asciinema"]), ["asciinema", "agg", "ffmpeg"]);
    assert_eq!(names(&["playwright"]), ["node", "ffmpeg", "playwright"]);
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
