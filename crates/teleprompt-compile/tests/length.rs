//! `timing.length_ms`: the whole video's length. Over it, `check` says by
//! how much, how much of the time pictures hold, and about how many words
//! of the rest of the narration to cut (docs/design.md#led-by-the-picture).

use std::path::Path;

use teleprompt_compile::VoiceContext;
use teleprompt_compile::{compile, CompileOutput};
use teleprompt_core::config::PartialConfig;
use teleprompt_scene::ScenePlugins;
use teleprompt_script::parse::parse_script;
use teleprompt_script::program::resolve;
use teleprompt_voice::cache::VoiceCache;
use teleprompt_voice::WpmEstimator;

const BODY: &str = "\
# Setting up

Welcome to Acme. Let me show you around, from the first step to the last. {#welcome}

# Deploying

Deployment is one command, and it streams progress as it goes. {#deploy}

```teleprompt scene=mock policy=fit-line
wait 6000ms
```
";

fn compile_with(length_ms: u64) -> CompileOutput {
    let src = format!("---\ntiming:\n  length_ms: {length_ms}\n---\n\n{BODY}");
    let cache =
        VoiceCache::new(std::env::temp_dir().join(format!("tp-length-{}", std::process::id())));
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
        takes: &teleprompt_voice::takes::Takes::default(),
    };
    let parsed = parse_script(&src).expect("parses");
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
fn a_video_within_its_length_says_nothing() {
    let out = compile_with(600_000);
    assert!(
        out.warnings.iter().all(|w| !w.contains("length")),
        "{:?}",
        out.warnings
    );
}

#[test]
fn a_video_over_its_length_says_what_is_fixed_and_what_to_cut() {
    let total = compile_with(600_000).timeline.duration_ms;
    let out = compile_with(total.ms() - 2000);
    let w = out
        .warnings
        .iter()
        .find(|w| w.contains("length"))
        .unwrap_or_else(|| panic!("{:?}", out.warnings));
    // Over by two seconds; the fit-line picture holds its 6 s.
    assert!(w.contains("over by 2.0 s"), "{w}");
    assert!(w.contains("6.0 s is held by pictures (`deploy`)"), "{w}");
    // The only other narration is `welcome`'s 15 words.
    assert!(w.contains("of its 15 words"), "{w}");
    // Chapter by chapter, where the time goes.
    assert!(w.contains("Setting up") && w.contains("Deploying"), "{w}");
}
