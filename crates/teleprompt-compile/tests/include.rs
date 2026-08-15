use std::path::{Path, PathBuf};

use teleprompt_compile::compile;
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::resolve;
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::NullVoice;

fn workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "teleprompt-include-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, src: &str) -> Result<teleprompt_compile::CompileOutput, Vec<String>> {
    let mut s = parse_script(src).unwrap();
    assign_ids(&mut s);
    let p = resolve(
        &s,
        "d.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap();
    compile(
        &p,
        &SceneRegistry::with_builtins(),
        &NullVoice::default(),
        dir,
        "0.1.0",
    )
    .map_err(|d| d.0.iter().map(|x| x.message.clone()).collect())
}

#[test]
fn an_included_file_supplies_the_block_body() {
    let dir = workspace();
    std::fs::write(dir.join("steps.mock"), "wait 700ms\n").unwrap();
    let out = run(
        &dir,
        "# A\n\nOne. {#a}\n\n```teleprompt scene=mock include=steps.mock\n```\n",
    )
    .unwrap();
    assert_eq!(
        out.timeline.entries[0].action.as_ref().unwrap().duration_ms,
        700
    );
}

#[test]
fn a_missing_include_is_a_diagnostic() {
    let dir = workspace();
    let e = run(
        &dir,
        "# A\n\nOne. {#a}\n\n```teleprompt scene=mock include=absent.mock\n```\n",
    )
    .unwrap_err();
    assert!(e[0].contains("cannot read included file `absent.mock`"));
}

#[test]
fn a_fence_with_both_a_body_and_an_include_is_an_error() {
    let dir = workspace();
    std::fs::write(dir.join("steps.mock"), "wait 700ms\n").unwrap();
    let e = run(
        &dir,
        "# A\n\nOne. {#a}\n\n```teleprompt scene=mock include=steps.mock\nwait 1s\n```\n",
    )
    .unwrap_err();
    assert!(e[0].contains("has both a body and an `include`"));
}

#[test]
fn an_include_escaping_the_project_root_is_refused() {
    let dir = workspace();
    let e = run(
        &dir,
        "# A\n\nOne. {#a}\n\n```teleprompt scene=mock include=../outside.mock\n```\n",
    )
    .unwrap_err();
    assert!(e[0].contains("outside the project"));
}
