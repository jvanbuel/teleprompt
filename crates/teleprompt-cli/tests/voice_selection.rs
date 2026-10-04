use std::collections::BTreeMap;
use std::path::PathBuf;

use teleprompt_cli::voice::{backends_for, Backends};

fn settings(yaml: &str) -> BTreeMap<String, serde_yaml::Value> {
    let mut m = BTreeMap::new();
    m.insert("kokoro".to_string(), serde_yaml::from_str(yaml).unwrap());
    m
}

#[test]
fn kokoro_is_registered_by_default() {
    assert_eq!(
        Backends::defaults().ids(),
        vec![
            "elevenlabs",
            "gemini",
            "kokoro",
            "null",
            "openai",
            "voicebox"
        ]
    );
}

#[test]
fn kokoro_takes_its_settings_from_the_backends_map() {
    let b = backends_for(
        &settings("base_url: \"http://gpu-box:8880\""),
        "teleprompt.toml",
    );
    b.resolve("kokoro")
        .unwrap_or_else(|d| panic!("{}", d.message));
    // The address used to be observable through the version string, which
    // is what keyed the cache. It is not any more — a key that names a
    // machine is a cache that never leaves it — so the backend's own
    // `address` is what proves the settings arrived.
    let k = b
        .resolve("kokoro")
        .expect("valid settings construct a backend");
    assert_eq!(k.address().as_deref(), Some("http://gpu-box:8880"));
}

/// A backend whose settings do not validate must still be *reported* —
/// deferring the failure to the projects that select it is not the same as
/// dropping it, and the message must survive the deferral intact.
#[test]
fn a_bad_backend_setting_is_reported_not_swallowed() {
    let b = backends_for(&settings("concurrency: 0"), "/p/teleprompt.toml");
    match b.resolve("kokoro") {
        Ok(_) => panic!("expected an error"),
        Err(d) => {
            assert!(d.message.contains("concurrency"), "{}", d.message);
            assert_eq!(
                d.file.as_deref(),
                Some("/p/teleprompt.toml"),
                "a settings error belongs to the settings file"
            );
        }
    }
}

/// I2, the other half. The same bad setting must not stop a project that
/// resolves to a different backend, which is the offline inner loop the
/// whole delivery exists to keep fast and network-free.
#[test]
fn a_bad_setting_for_one_backend_leaves_the_others_usable() {
    let b = backends_for(&settings("concurrency: 0"), "teleprompt.toml");
    assert!(b.resolve("null").is_ok());
    assert!(
        b.diagnostics().is_empty(),
        "a shipped backend with bad settings is not an unknown backend"
    );
    assert_eq!(
        b.ids(),
        vec![
            "elevenlabs",
            "gemini",
            "kokoro",
            "null",
            "openai",
            "voicebox"
        ],
        "a misconfigured backend is still one this build ships"
    );
}

/// A `backends:` key naming no shipped voice is a server of the author's;
/// one that does not make a server, such as a misspelt `[backends.kokoro]`,
/// is an error rather than settings nothing reads.
#[test]
fn settings_for_a_backend_this_build_lacks_are_named_not_dropped() {
    let mut m = BTreeMap::new();
    m.insert(
        "kokoro-local".to_string(),
        serde_yaml::from_str("voice: af_heart").unwrap(),
    );
    let b = backends_for(&m, "/p/teleprompt.toml");

    let diags = b.diagnostics();
    assert_eq!(diags.len(), 1, "one diagnostic per unmatched key");
    assert!(diags[0].is_error(), "{}", diags[0].message);
    assert!(
        diags[0].message.contains("backends.kokoro-local"),
        "{}",
        diags[0].message
    );
    assert!(
        diags[0].message.contains("kokoro") && diags[0].message.contains("null"),
        "must list the ids this build ships: {}",
        diags[0].message
    );
    assert_eq!(diags[0].file.as_deref(), Some("/p/teleprompt.toml"));
}

/// A block with an address is a server of the author's, under the name
/// they gave it.
#[test]
fn a_block_naming_no_shipped_voice_is_a_server() {
    let mut m = BTreeMap::new();
    m.insert(
        "studio".to_string(),
        serde_yaml::from_str("base_url: \"http://127.0.0.1:8881/v1\"\nmodel: piper").unwrap(),
    );
    let b = backends_for(&m, "teleprompt.toml");
    assert!(
        b.diagnostics().is_empty(),
        "{:?}",
        b.diagnostics()[0].message
    );
    let studio = b
        .resolve("studio")
        .unwrap_or_else(|d| panic!("{}", d.message));
    assert_eq!(studio.id(), "studio");
    assert_eq!(studio.version(), "piper");
    assert!(b.ids().contains(&"studio".to_string()));
}

/// An endpoint without an address is not usable, and says what it lacks.
#[test]
fn an_endpoint_without_an_address_says_so() {
    let mut m = BTreeMap::new();
    m.insert(
        "studio".to_string(),
        serde_yaml::from_str("model: piper").unwrap(),
    );
    let b = backends_for(&m, "teleprompt.toml");
    let why = b.resolve("studio").err().unwrap().message;
    assert!(why.contains("base_url"), "{why}");
}

/// Groups 2 and 3 must compose: an unknown key belongs to no resolvable
/// backend, so a fix that only validates the selected backend must not
/// silence it.
#[test]
fn an_unknown_key_is_reported_even_when_the_project_resolves_elsewhere() {
    let mut m = BTreeMap::new();
    m.insert(
        "kokoro-local".to_string(),
        serde_yaml::from_str("voice: af_heart").unwrap(),
    );
    m.insert(
        "kokoro".to_string(),
        serde_yaml::from_str("concurrency: 0").unwrap(),
    );
    let b = backends_for(&m, "teleprompt.toml");

    assert!(b.resolve("null").is_ok(), "null is unaffected by either");
    assert_eq!(b.diagnostics().len(), 1, "the unknown key still reports");
}

/// Settings that never produced a backend must not leave one behind, or
/// `dub`'s fan-out limit and voice-list check, and `setup`'s probe, would
/// silently read from (or probe) an `OpenAiVoice` built from defaults
/// instead of the broken settings the author actually wrote.
#[test]
fn no_backend_is_left_when_settings_do_not_validate() {
    let b = backends_for(&settings("concurrency: 0"), "teleprompt.toml");
    assert!(
        b.resolve("kokoro").is_err(),
        "unusable settings must not leave a backend for dub or setup to read from"
    );
    assert_eq!(b.concurrency("kokoro"), 1);
}

/// The value `dub`'s fan-out limit reads is the same `concurrency` the
/// project's settings named — not a value `OpenAiVoice` decided on its own
/// after the fact.
#[test]
fn kokoro_carries_the_configured_concurrency() {
    let b = backends_for(&settings("concurrency: 7"), "teleprompt.toml");
    assert_eq!(b.concurrency("kokoro"), 7);
}

/// A backend with no server, such as `null`, has nothing for `setup` to
/// probe, whichever others constructed.
#[test]
fn null_has_no_server_to_probe() {
    let b = backends_for(
        &settings("base_url: \"http://gpu-box:8880\""),
        "teleprompt.toml",
    );
    assert_eq!(b.resolve("null").unwrap().address(), None);
}

fn tempdir(tag: &str) -> teleprompt_testkit::TestDir {
    teleprompt_testkit::test_dir(&format!("voice-selection-{tag}"))
}

const SCRIPT_WITH_FRONT_MATTER_BACKENDS: &str = "\
---
backends:
  kokoro:
    base_url: \"http://gpu-box:8880\"
---

# Quick start

Every video in this repository is built from a script you can read.
";

/// A script's own front matter can carry `backends:` too — `Config`'s doc
/// comment says so, and `resolve` genuinely merges it into
/// `program.config.backends`. But the registry `Project::compile` builds is
/// constructed from the *project's* `backends:` alone, before this script
/// is ever read, so the override the merge computed is never seen by
/// anything that could act on it. That must be named, not dropped.
#[test]
fn a_front_matter_backends_override_is_reported_not_silently_dropped() {
    let dir = tempdir("front-matter-backends");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(
        dir.join("scripts/test.md"),
        SCRIPT_WITH_FRONT_MATTER_BACKENDS,
    )
    .unwrap();
    let project = teleprompt_cli::project::Project::discover(&dir).unwrap();
    let script = dir.join("scripts/test.md");

    let warnings =
        teleprompt_cli::cmd::check::run_check(&project, &script, "en").unwrap_or_else(|e| {
            panic!("a dropped front-matter override must not fail the compile: {e:?}")
        });

    let msg = warnings
        .iter()
        .find(|w| w.contains("kokoro"))
        .unwrap_or_else(|| panic!("expected a warning naming kokoro, got {warnings:?}"));
    assert!(msg.contains("teleprompt.toml"), "{msg}");
    assert!(
        msg.contains("no effect") || msg.contains("not applied"),
        "{msg}"
    );

    // This warning used to be pushed already rendered — carrying its own
    // `warning: ` prefix and a trailing `\n  --> file` line — into a vec whose
    // only consumers (`main.rs`'s `eprintln!("warning: {w}")` and `--format
    // json`'s `warnings` array) add that framing themselves. That doubled the
    // prefix on stderr and, under `--format json`, put a multi-line,
    // already-labelled string next to every other entry's bare sentence.
    assert!(
        !msg.starts_with("warning:"),
        "the caller adds the `warning: ` prefix; a bare warning must not \
         already carry one or printed output doubles it: {msg}"
    );
    assert!(
        !msg.contains('\n'),
        "warnings are one line each, like every other entry in this vec \
         (cache, scheduling); an embedded `--> file` line breaks that \
         shape and the JSON array's homogeneity: {msg}"
    );
}

/// The negative case: a project whose `teleprompt.toml` already sets the
/// same `backends.kokoro` value the script's front matter repeats must not
/// warn — the merge genuinely agrees with what the registry was built
/// from, so there is nothing dropped to report.
#[test]
fn a_front_matter_backends_block_matching_the_project_is_not_a_warning() {
    let dir = tempdir("front-matter-backends-match");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(
        dir.join("teleprompt.toml"),
        "[backends.kokoro]\nbase_url = \"http://gpu-box:8880\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("scripts/test.md"),
        SCRIPT_WITH_FRONT_MATTER_BACKENDS,
    )
    .unwrap();
    let project = teleprompt_cli::project::Project::discover(&dir).unwrap();
    let script = dir.join("scripts/test.md");

    let warnings = teleprompt_cli::cmd::check::run_check(&project, &script, "en").unwrap();
    assert!(
        !warnings.iter().any(|w| w.contains("kokoro")),
        "{warnings:?}"
    );
}

/// A scaffolded project plus a `teleprompt.toml` of the caller's choosing.
/// The scaffold's own `voice.backend` is `null`, which is what makes the
/// "settings for a backend this project never uses" case reachable.
fn project_with_toml(
    tag: &str,
    toml: &str,
) -> (
    teleprompt_testkit::TestDir,
    teleprompt_cli::project::Project,
    PathBuf,
) {
    let dir = tempdir(tag);
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(dir.join("teleprompt.toml"), toml).unwrap();
    let project = teleprompt_cli::project::Project::discover(&dir).unwrap();
    let script = dir.join("scripts/demo.md");
    (dir, project, script)
}

const NULL_PROJECT: &str = "\
[locales]
source = \"en\"
targets = []

[voice]
backend = \"null\"

[scene.mock]
plugin = \"mock\"
";

const KOKORO_PROJECT: &str = "\
[locales]
source = \"en\"
targets = []

[voice]
backend = \"kokoro\"

[scene.mock]
plugin = \"mock\"
";

/// I2, end to end. `check` and `plan` are the offline inner loop: a project
/// that resolves to `null` never opens a socket and never reads a Kokoro
/// setting, so a bad one must not fail it. It used to, with exit 2 and no
/// file anchor, because every backend was constructed eagerly.
#[test]
fn a_bad_setting_for_a_backend_the_project_never_uses_does_not_fail_check() {
    let (_dir, project, script) = project_with_toml(
        "unused-bad-setting",
        &format!("{NULL_PROJECT}\n[backends.kokoro]\nconcurrency = 0\n"),
    );
    teleprompt_cli::cmd::check::run_check(&project, &script, "en").unwrap_or_else(|e| {
        panic!("a null-backend project must not read kokoro's settings: {e:?}")
    });
}

/// The same setting on a project that *does* resolve to kokoro still fails,
/// and now says which file to go and fix.
#[test]
fn a_bad_setting_for_the_selected_backend_fails_check_and_names_the_config_file() {
    let (_dir, project, script) = project_with_toml(
        "used-bad-setting",
        &format!("{KOKORO_PROJECT}\n[backends.kokoro]\nconcurrency = 0\n"),
    );
    let errors = teleprompt_cli::cmd::check::run_check(&project, &script, "en")
        .expect_err("a backend the project uses must validate");
    let joined = errors.join("\n");
    assert!(joined.contains("concurrency"), "{joined}");
    assert!(
        joined.contains("teleprompt.toml"),
        "the only check diagnostic with no file anchor was this one: {joined}"
    );
}

/// I1, end to end. A near-miss spelling of a shipped backend id with no
/// address reached nothing and said nothing, and `dub` then connected to
/// the default server.
#[test]
fn an_unknown_backends_key_fails_check_and_names_the_key() {
    let (_dir, project, script) = project_with_toml(
        "unknown-backends-key",
        &format!("{NULL_PROJECT}\n[backends.kokoro-local]\nvoice = \"af_heart\"\n"),
    );
    let errors = teleprompt_cli::cmd::check::run_check(&project, &script, "en")
        .expect_err("settings that reach nothing must be named");
    let joined = errors.join("\n");
    assert!(joined.contains("backends.kokoro-local"), "{joined}");
    assert!(joined.contains("teleprompt.toml"), "{joined}");
}

/// The two fixes compose. Validating only the selected backend must not
/// become a way for an unmatched key to slip through, and an unmatched key
/// must not resurrect the eager validation of an unused one.
#[test]
fn an_unknown_key_and_an_unused_bad_setting_report_exactly_one_problem() {
    let (_dir, project, script) = project_with_toml(
        "compose",
        &format!(
            "{NULL_PROJECT}\n[backends.kokoro]\nconcurrency = 0\n\
             \n[backends.kokoro-local]\nvoice = \"af_heart\"\n"
        ),
    );
    let errors = teleprompt_cli::cmd::check::run_check(&project, &script, "en")
        .expect_err("the unknown key is still an error");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("backends.kokoro-local"), "{errors:?}");
    assert!(
        !errors[0].contains("concurrency"),
        "a backend this project never uses must not be validated: {errors:?}"
    );
}

/// Gemini's key is read when a line is spoken, not when the project is
/// checked: `check` stays offline and needs no secret.
#[test]
fn a_gemini_project_checks_without_a_key() {
    let (_dir, project, script) = project_with_toml(
        "gemini-no-key",
        &NULL_PROJECT.replace(
            "backend = \"null\"",
            "backend = \"gemini\"\n\n[backends.gemini]\napi_key_env = \"TELEPROMPT_TEST_NO_SUCH_KEY\"",
        ),
    );
    teleprompt_cli::cmd::check::run_check(&project, &script, "en")
        .unwrap_or_else(|e| panic!("check must not need Gemini's key: {e:?}"));
}

/// `dub` sends Gemini as many lines at once as its settings say.
#[test]
fn gemini_carries_its_configured_concurrency() {
    let mut s = BTreeMap::new();
    s.insert(
        "gemini".to_string(),
        serde_yaml::from_str("concurrency: 2").unwrap(),
    );
    assert_eq!(backends_for(&s, "teleprompt.toml").concurrency("gemini"), 2);
}
