//! `voice.instruct`: how a line is to be delivered, sent to a backend that
//! takes instructions and part of the line's cache key.

use std::path::Path;

use teleprompt_core::config::PartialConfig;
use teleprompt_pipeline::compile::{compile, CompileOutput, VoiceContext};
use teleprompt_scene::ScenePlugins;
use teleprompt_script::parse::parse_script;
use teleprompt_script::program::resolve;
use teleprompt_voice::cache::VoiceCache;
use teleprompt_voice::WpmEstimator;

fn compile_it(src: &str) -> CompileOutput {
    let cache =
        VoiceCache::new(std::env::temp_dir().join(format!("tp-instruct-{}", std::process::id())));
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
        takes: &teleprompt_voice::takes::Takes::default(),
    };
    let parsed = parse_script(src).expect("parses");
    let program = resolve(
        &parsed,
        "t.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap();
    compile(
        &program,
        &ScenePlugins::mock(),
        &ctx,
        Path::new("."),
        "0.1.0",
    )
    .unwrap()
}

#[test]
fn a_script_default_and_a_lines_own_instructions_reach_the_request() {
    let out = compile_it(
        "---\nvoice:\n  instruct: calmly\n---\n\n# A\n\n\
         One. {#one}\n\nTwo. {#two voice.instruct=\"warmly, with a smile\"}\n",
    );
    let instruct: Vec<Option<&str>> = out
        .narration
        .iter()
        .map(|d| d.synth_request.instruct.as_deref())
        .collect();
    assert_eq!(instruct, [Some("calmly"), Some("warmly, with a smile")]);
    let plain = compile_it("# A\n\nOne. {#one}\n");
    assert_eq!(plain.narration[0].synth_request.instruct, None);
    assert_ne!(plain.narration[0].cache_key, out.narration[0].cache_key);
}
