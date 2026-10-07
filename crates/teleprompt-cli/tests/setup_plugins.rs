//! Every program a scene plugin says it is missing is one `teleprompt setup
//! <plugin>` knows how to install: the two lists cannot drift apart.

use teleprompt_core::tool::programs_in;

#[test]
fn setup_covers_every_program_an_plugin_finds_missing() {
    // Nothing on PATH, so every scene plugin that runs a program says which. The
    // only test in this binary, so no other reads PATH meanwhile.
    let empty = teleprompt_testkit::test_dir("setup-plugins");
    std::env::set_var("PATH", empty.path());
    let mut checked = 0;
    for p in teleprompt_cli::registry::registry().scenes.iter() {
        let (plugin, backend) = (p.name(), p.capture());
        // Held back by something setup cannot install (macos off a Mac).
        let Some(why) = backend.unavailable() else {
            continue;
        };
        let Some(programs) = programs_in(&why) else {
            continue;
        };
        let known: Vec<&str> =
            teleprompt_cli::registry::setup_here(teleprompt_cli::registry::registry())
                .resolve(&[plugin.to_string()])
                .unwrap()
                .iter()
                .map(|t| t.name)
                .collect();
        for program in programs {
            assert!(
                known.contains(&program),
                "{plugin} runs {program}, which setup does not know"
            );
            checked += 1;
        }
    }
    assert!(checked >= 6, "only {checked} programs were checked");
}
