//! Every program an adapter says it is missing is one `teleprompt setup
//! <adapter>` knows how to install: the two lists cannot drift apart.

use teleprompt_capture::tool::NOT_ON_PATH;
use teleprompt_cli::cmd::setup::resolve;

#[test]
fn setup_covers_every_program_an_adapter_finds_missing() {
    // Nothing on PATH, so every adapter that runs a program says which. The
    // only test in this binary, so no other reads PATH meanwhile.
    let empty = teleprompt_testkit::test_dir("setup-adapters");
    std::env::set_var("PATH", empty.path());
    let mut checked = 0;
    for (adapter, backend) in teleprompt_cli::scene::captures().backends() {
        // Held back by something setup cannot install (macos off a Mac).
        let Some(why) = backend.unavailable().filter(|w| w.ends_with(NOT_ON_PATH)) else {
            continue;
        };
        let known: Vec<&str> = resolve(&[adapter.to_string()])
            .unwrap()
            .iter()
            .map(|t| t.name)
            .collect();
        for program in why.trim_end_matches(NOT_ON_PATH).split(" and ") {
            assert!(
                known.contains(&program),
                "{adapter} runs {program}, which setup does not know"
            );
            checked += 1;
        }
    }
    assert!(checked >= 6, "only {checked} programs were checked");
}
