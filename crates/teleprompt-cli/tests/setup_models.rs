//! A model `teleprompt setup` installed is the one commands listen with,
//! unless `--model` names another; with none, they say how to get one.
#![cfg(feature = "listen")]

use std::process::{Command, Output};

const SPEECH: &str = "sherpa-onnx-streaming-zipformer-en-2023-06-26";

fn tp(models: &std::path::Path, args: &[&str]) -> Output {
    let dir = teleprompt_testkit::test_dir("setup-models-project");
    teleprompt::new::scaffold(&dir).unwrap();
    Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(&dir)
        .env("TELEPROMPT_MODELS", models)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn without_a_model_prompt_and_record_say_how_to_install_one() {
    let models = teleprompt_testkit::test_dir("setup-models-none");
    for args in [
        &["serve", "scripts/demo.md", "--port", "0"][..],
        &["record", "scripts/new.md"][..],
    ] {
        let out = tp(&models, args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {err}");
        assert!(
            err.contains("teleprompt setup speech-model"),
            "{args:?}: {err}"
        );
    }
}

/// An installed model is loaded: this one is empty, so loading it fails,
/// naming it.
#[test]
fn an_installed_model_is_used_without_naming_it() {
    let models = teleprompt_testkit::test_dir("setup-models-some");
    std::fs::create_dir(models.join(SPEECH)).unwrap();
    let out = tp(&models, &["serve", "scripts/demo.md", "--port", "0"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    let installed = models.join(SPEECH).display().to_string();
    assert!(
        err.contains(&installed) && !err.contains("teleprompt setup"),
        "{err}"
    );
}
