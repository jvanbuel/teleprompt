//! A script compiled for another locale speaks its translation, from the
//! `.<locale>.yaml` beside it.

use std::path::Path;
use std::process::{Command, Output};

use teleprompt_script::translation::{source_of, Entry, Translation};

const SCRIPT: &str = "\
# Introduction

Welcome to Acme. {#welcome}

```teleprompt scene=mock policy=concurrent
wait 300ms
```

Deploying is one command. {#deploy}
";

fn project(tag: &str) -> teleprompt_testkit::TestDir {
    let dir = teleprompt_testkit::test_dir(&format!("locale-{tag}"));
    teleprompt::new::scaffold(&dir).unwrap();
    std::fs::write(dir.join("scripts/tour.md"), SCRIPT).unwrap();
    dir
}

fn dutch(stale: bool) -> String {
    let e = |en: &str, nl: &str| Entry {
        from: source_of(en),
        text: nl.to_string(),
    };
    let mut t = Translation::default();
    t.chapters
        .push(("introduction".into(), e("Introduction", "Inleiding")));
    let welcome = if stale {
        "Welcome, all."
    } else {
        "Welcome to Acme."
    };
    t.lines
        .push(("welcome".into(), e(welcome, "Welkom bij Acme.")));
    t.lines.push((
        "deploy".into(),
        e("Deploying is one command.", "Uitrollen is één commando."),
    ));
    t.to_yaml("tour.md", "nl")
}

fn tp(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn a_translated_script_plans_and_dubs_in_its_locale() {
    let dir = project("dub");
    std::fs::write(dir.join("scripts/tour.nl.yaml"), dutch(false)).unwrap();
    let out = tp(
        &dir,
        &["dub", "scripts/tour.md", "--locale", "nl", "--out", "out"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let manifest = std::fs::read_to_string(dir.join("out/nl/narration.json")).unwrap();
    assert!(
        manifest.contains("Uitrollen is één commando."),
        "{manifest}"
    );
    assert!(manifest.contains("\"title\": \"Inleiding\""), "{manifest}");
    let vtt = std::fs::read_to_string(dir.join("out/nl/captions.vtt")).unwrap();
    assert!(vtt.contains("Welkom bij Acme."), "{vtt}");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("changed"));
}

#[test]
fn the_source_locale_needs_no_translation() {
    let dir = project("source");
    let out = tp(&dir, &["plan", "scripts/tour.md", "--format", "json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_locale_with_no_translation_says_how_to_make_one() {
    let dir = project("missing");
    let out = tp(&dir, &["plan", "scripts/tour.md", "--locale", "nl"]);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("tour.nl.yaml") && err.contains("teleprompt translate"),
        "{err}"
    );
}

#[test]
fn a_stale_translation_is_spoken_and_reported() {
    let dir = project("stale");
    std::fs::write(dir.join("scripts/tour.nl.yaml"), dutch(true)).unwrap();
    let out = tp(&dir, &["plan", "scripts/tour.md", "--locale", "nl"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // `plan` prints its warnings with the plan, on stdout.
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("`welcome`") && said.contains("changed"),
        "{said}"
    );
}

/// `check --locale` lints what is said in that locale: the translation.
#[test]
fn check_lints_the_translation() {
    let dir = project("lint");
    let doubled = dutch(false).replace("één commando.", "één commando commando.");
    std::fs::write(dir.join("scripts/tour.nl.yaml"), doubled).unwrap();
    let out = tp(
        &dir,
        &[
            "check",
            "scripts/tour.md",
            "--locale",
            "nl",
            "--format",
            "json",
        ],
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["ok"], true, "{report}");
    let warnings = report["warnings"].to_string();
    assert!(warnings.contains("commando commando"), "{warnings}");
}

/// A take is of the English: the translated line is synthesized, while the
/// English video still speaks the take.
#[test]
fn an_english_take_is_not_spoken_in_a_translation() {
    let dir = project("takes");
    std::fs::write(dir.join("scripts/tour.nl.yaml"), dutch(false)).unwrap();
    let project =
        teleprompt::project::Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let mut takes = teleprompt_voice::takes::Takes::load(&project.takes_dir()).unwrap();
    let pcm = teleprompt_voice::Pcm {
        sample_rate: 24_000,
        channels: 1,
        samples: vec![0; 24_000],
    };
    takes.save("welcome", "Welcome to Acme.", &pcm).unwrap();

    let recorded = |locale: &str| {
        let out = tp(
            &dir,
            &[
                "plan",
                "scripts/tour.md",
                "--locale",
                locale,
                "--format",
                "json",
            ],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let timeline: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        // `recorded` is left out when false.
        timeline["entries"][0]["narration"]["recorded"] == true
    };
    assert!(recorded("en"));
    assert!(!recorded("nl"));
}

/// A project written in Dutch is compiled in Dutch unless told otherwise:
/// `--locale` defaults to the project's `locales.source`, not to `en`.
#[test]
fn a_project_s_own_language_is_the_default_locale() {
    let dir = project("source-nl");
    let toml = std::fs::read_to_string(dir.join("teleprompt.toml")).unwrap();
    std::fs::write(
        dir.join("teleprompt.toml"),
        toml.replace("source = \"en\"", "source = \"nl\""),
    )
    .unwrap();
    for args in [
        &["check", "scripts/tour.md"][..],
        &["plan", "scripts/tour.md"],
        &[
            "edit",
            "scripts/tour.md",
            "stretch",
            "welcome-a",
            "--by",
            "2",
        ],
    ] {
        let out = tp(&dir, args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// A locale names a file beside the script and a directory under `--out`,
/// so one that could name anything else is refused before it is used.
#[test]
fn a_locale_that_is_not_a_language_tag_is_refused() {
    let dir = project("bad-locale");
    for locale in ["../../zz", "nl/x", "", "a b"] {
        let out = tp(
            &dir,
            &[
                "--format",
                "json",
                "check",
                "scripts/tour.md",
                "--locale",
                locale,
            ],
        );
        assert_eq!(out.status.code(), Some(2), "{locale:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!err.contains("panicked"), "{err}");
    }
}
