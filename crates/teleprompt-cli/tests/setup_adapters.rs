//! Every program an adapter says it is missing is one `teleprompt setup
//! <adapter>` knows how to install: the two lists cannot drift apart.

use std::process::Command;

use teleprompt_cli::cmd::setup::resolve;

#[test]
fn setup_covers_every_program_doctor_finds_missing() {
    let empty = teleprompt_testkit::test_dir("setup-adapters");
    let out = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["--format", "json", "doctor"])
        .current_dir(empty.path())
        .env("PATH", empty.path())
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut checked = 0;
    for backend in report["capture_backends"].as_array().unwrap() {
        let adapter = backend["adapter"].as_str().unwrap();
        let Some(why) = backend["unavailable"].as_str() else {
            continue;
        };
        let known: Vec<&str> = resolve(&[adapter.to_string()])
            .unwrap()
            .iter()
            .map(|t| t.name)
            .collect();
        for program in why.trim_end_matches(" is not on PATH").split(" and ") {
            assert!(
                known.contains(&program),
                "{adapter} runs {program}, which setup does not know"
            );
            checked += 1;
        }
        // What doctor tells the author to do about it.
        assert_eq!(
            backend["fix"],
            format!("teleprompt setup {adapter}"),
            "{backend}"
        );
    }
    assert!(checked >= 6, "{report}");
}
