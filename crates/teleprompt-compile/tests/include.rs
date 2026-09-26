use std::path::Path;

use teleprompt_cache::VoiceCache;
use teleprompt_compile::{compile, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::resolve;
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::WpmEstimator;

fn workspace() -> teleprompt_testkit::TestDir {
    teleprompt_testkit::test_dir("include")
}

/// A cache rooted alongside `workspace()`'s scratch directory rather than
/// inside it, so it is never mistaken for a script include. Only ever read
/// in this file, so a cold cache every time is fine.
fn cache_for(dir: &Path) -> VoiceCache {
    VoiceCache::new(dir.with_extension("voice-cache"))
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
    let cache = cache_for(dir);
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
        takes: &teleprompt_voice::takes::Takes::default(),
    };
    compile(&p, &SceneRegistry::with_builtins(), &ctx, dir, "0.1.0")
        .map_err(|d| d.0.iter().map(|x| x.message.clone()).collect())
}

/// Like `run`, but keeps the whole rendered diagnostic — file, line, column —
/// rather than just the message, as the CLI prints it.
fn run_rendered(dir: &Path, script_name: &str, src: &str) -> Result<(), Vec<String>> {
    let mut s = parse_script(src).unwrap();
    assign_ids(&mut s);
    let p = resolve(
        &s,
        script_name,
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap();
    let cache = cache_for(dir);
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
        takes: &teleprompt_voice::takes::Takes::default(),
    };
    compile(&p, &SceneRegistry::with_builtins(), &ctx, dir, "0.1.0")
        .map(|_| ())
        .map_err(|d| d.0.iter().map(|x| x.render(script_name)).collect())
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

/// `BlockSource` used to carry only the fence's shot, and the mock adapter
/// offset body line `i` by `shot.line + i + 1`. Right for an inline body;
/// meaningless for an included one — an error on line 3 of `steps.mock` came
/// out as `scripts/h1.md:15:1` in a thirteen-line script. Ruling F12 removed a
/// fabricated `line: 0` from this exact path; `include=` reintroduced a
/// fabricated line by another route.
#[test]
fn a_bad_directive_in_an_included_file_names_that_file_and_its_own_line() {
    let dir = workspace();
    std::fs::write(
        dir.join("steps.mock"),
        "wait 100ms\nmark\nbogus directive here\n",
    )
    .unwrap();

    let errs = run_rendered(
        &dir,
        "scripts/h1.md",
        // The fence sits well down the script, so a fence-relative offset
        // would produce a visibly different (and out-of-range) line.
        "# Include\n\nHello there world. {#hi}\n\n\n\n\n\n```teleprompt scene=mock include=steps.mock\n```\n",
    )
    .unwrap_err();

    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(
        errs[0].contains("unknown mock directive `bogus`"),
        "{}",
        errs[0]
    );
    assert!(
        errs[0].contains("steps.mock:3:1"),
        "the included file's own path and line, not the script's: {}",
        errs[0]
    );
    assert!(
        !errs[0].contains("h1.md"),
        "the script is not where this line lives: {}",
        errs[0]
    );
}

/// The other half: an inline body's diagnostics still point at the script,
/// with the fence offset applied exactly as before.
#[test]
fn an_inline_body_still_reports_the_scripts_own_path_and_offset_line() {
    let dir = workspace();
    let errs = run_rendered(
        &dir,
        "scripts/h1.md",
        "# Inline\n\nHello there world. {#hi}\n\n```teleprompt scene=mock\nwait 100ms\nbogus directive here\n```\n",
    )
    .unwrap_err();

    assert_eq!(errs.len(), 1, "{errs:?}");
    // The fence opens on script line 5; body line 1 (0-based) is line 7.
    assert!(
        errs[0].contains("scripts/h1.md:7:1"),
        "inline bodies keep today's behaviour: {}",
        errs[0]
    );
}

/// `include=file#fragment` reads the file and leaves the fragment to the
/// adapter. One that does not take fragments says so, naming itself,
/// rather than reading the whole file as if the fragment were not there.
#[test]
fn a_fragment_is_the_adapters_to_take_or_refuse() {
    let dir = workspace();
    std::fs::write(dir.join("steps.mock"), "wait 700ms\n").unwrap();
    let e = run(
        &dir,
        "# A\n\nOne. {#a}\n\n```teleprompt scene=mock include=steps.mock#2\n```\n",
    )
    .unwrap_err();
    assert!(
        e[0].contains("`mock` blocks do not take an `include=…#2`"),
        "{e:?}"
    );
}

/// The path before the `#` is checked like any include path.
#[test]
fn a_fragment_does_not_hide_an_escaping_path() {
    let dir = workspace();
    let e = run(
        &dir,
        "# A\n\nOne. {#a}\n\n```teleprompt scene=mock include=../outside.mock#1\n```\n",
    )
    .unwrap_err();
    assert!(
        e[0].contains("`../outside.mock` resolves outside the project"),
        "{e:?}"
    );
}
