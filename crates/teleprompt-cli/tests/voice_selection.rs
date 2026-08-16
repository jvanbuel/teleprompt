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
    assert_eq!(Backends::defaults().ids(), vec!["kokoro", "null"]);
}

#[test]
fn kokoro_takes_its_settings_from_the_backends_map() {
    let b = backends_for(
        &settings("base_url: \"http://gpu-box:8880\""),
        "teleprompt.toml",
    );
    let k = b
        .resolve("kokoro")
        .unwrap_or_else(|d| panic!("{}", d.message));
    // The version string is what keys the cache, so this is the observable
    // that proves the settings reached the backend rather than the default.
    assert!(
        k.capabilities().version.contains("gpu-box"),
        "{}",
        k.capabilities().version
    );
}

/// I2. A backend whose settings do not validate must still be *reported* —
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
        vec!["kokoro", "null"],
        "a misconfigured backend is still one this build ships"
    );
}

/// I1. A `backends:` key matching no shipped id used to be dropped without a
/// word, so `[backends.kokoro-local]` left `dub` talking to the default
/// `localhost:8880`.
#[test]
fn settings_for_a_backend_this_build_lacks_are_named_not_dropped() {
    let mut m = BTreeMap::new();
    m.insert(
        "kokoro-local".to_string(),
        serde_yaml::from_str("base_url: \"http://127.0.0.1:8881\"").unwrap(),
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

/// Groups 2 and 3 must compose: an unknown key belongs to no resolvable
/// backend, so a fix that only validates the selected backend must not
/// silence it.
#[test]
fn an_unknown_key_is_reported_even_when_the_project_resolves_elsewhere() {
    let mut m = BTreeMap::new();
    m.insert(
        "kokoro-local".to_string(),
        serde_yaml::from_str("base_url: \"http://127.0.0.1:8881\"").unwrap(),
    );
    m.insert(
        "kokoro".to_string(),
        serde_yaml::from_str("concurrency: 0").unwrap(),
    );
    let b = backends_for(&m, "teleprompt.toml");

    assert!(b.resolve("null").is_ok(), "null is unaffected by either");
    assert_eq!(b.diagnostics().len(), 1, "the unknown key still reports");
}

/// Group 6: `Backends::kokoro` is the escape hatch that replaced
/// `backend.as_any().downcast_ref::<KokoroVoice>()` at the three call sites
/// in `dub` and `doctor` that need an inherent method `VoiceBackend` does
/// not carry. It must gate on the same thing the downcast implicitly gated
/// on via `id()` matching plus a successful construction: asking under the
/// wrong id, or asking when settings never produced a backend, must both
/// come back empty rather than handing back a Kokoro server nothing
/// resolved to.
#[test]
fn the_kokoro_handle_is_gated_on_the_resolved_id() {
    let b = backends_for(
        &settings("base_url: \"http://gpu-box:8880\""),
        "teleprompt.toml",
    );
    assert!(
        b.kokoro("kokoro").is_some(),
        "kokoro constructed and was asked for by its own id"
    );
    assert!(
        b.kokoro("null").is_none(),
        "a project resolving to null must not reach kokoro's server just because kokoro also constructed"
    );
    assert!(b.kokoro("nonexistent-backend").is_none());
}

/// The other half of the same gate: settings that never produced a backend
/// must not leave a handle behind either, or `dub`'s fan-out limit and
/// voice-list check, and `doctor`'s probe, would silently read from (or
/// probe) a `KokoroVoice` built from defaults instead of the broken
/// settings the author actually wrote.
#[test]
fn the_kokoro_handle_is_absent_when_settings_do_not_validate() {
    let b = backends_for(&settings("concurrency: 0"), "teleprompt.toml");
    assert!(
        b.kokoro("kokoro").is_none(),
        "unusable settings must not leave a handle for dub or doctor to read from"
    );
}

/// The value `dub`'s fan-out limit reads is the same `concurrency` the
/// project's settings named — not a value `KokoroVoice` decided on its own
/// after the fact.
#[test]
fn the_kokoro_handle_carries_the_configured_concurrency() {
    let b = backends_for(&settings("concurrency: 7"), "teleprompt.toml");
    let k = b
        .kokoro("kokoro")
        .expect("valid settings construct a handle");
    assert_eq!(k.concurrency(), 7);
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "teleprompt-voice-selection-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
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
/// `program.config.backends`. But the registry `compile_script` builds is
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
fn project_with_toml(tag: &str, toml: &str) -> (teleprompt_cli::project::Project, PathBuf) {
    let dir = tempdir(tag);
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    std::fs::write(dir.join("teleprompt.toml"), toml).unwrap();
    let project = teleprompt_cli::project::Project::discover(&dir).unwrap();
    let script = dir.join("scripts/demo.md");
    (project, script)
}

const NULL_PROJECT: &str = "\
[locales]
source = \"en\"
targets = []

[voice]
source = \"synthetic\"
backend = \"null\"

[scene.mock]
adapter = \"mock\"
";

const KOKORO_PROJECT: &str = "\
[locales]
source = \"en\"
targets = []

[voice]
source = \"synthetic\"
backend = \"kokoro\"

[scene.mock]
adapter = \"mock\"
";

/// I2, end to end. `check` and `plan` are the offline inner loop: a project
/// that resolves to `null` never opens a socket and never reads a Kokoro
/// setting, so a bad one must not fail it. It used to, with exit 2 and no
/// file anchor, because every backend was constructed eagerly.
#[test]
fn a_bad_setting_for_a_backend_the_project_never_uses_does_not_fail_check() {
    let (project, script) = project_with_toml(
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
    let (project, script) = project_with_toml(
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

/// I1, end to end. A near-miss spelling of a shipped backend id reached
/// nothing and said nothing, and `dub` then connected to the default server.
#[test]
fn an_unknown_backends_key_fails_check_and_names_the_key() {
    let (project, script) = project_with_toml(
        "unknown-backends-key",
        &format!("{NULL_PROJECT}\n[backends.kokoro-local]\nbase_url = \"http://127.0.0.1:8881\"\n"),
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
    let (project, script) = project_with_toml(
        "compose",
        &format!(
            "{NULL_PROJECT}\n[backends.kokoro]\nconcurrency = 0\n\
             \n[backends.kokoro-local]\nbase_url = \"http://127.0.0.1:8881\"\n"
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
