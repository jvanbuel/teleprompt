//! `stretch=`: a block's shots run longer or shorter than their own pace,
//! as the author says. Only shots that state their length can be.

use std::path::Path;
use teleprompt_core::SpanMs;

use teleprompt_cache::VoiceCache;
use teleprompt_compile::{compile, CompileOutput, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::resolve;
use teleprompt_core::Diagnostics;
use teleprompt_plugin::ScenePlugins;
use teleprompt_voice::WpmEstimator;

fn compile_it(body: &str) -> Result<CompileOutput, Diagnostics> {
    let src = format!("# A\n\n{body}");
    let cache =
        VoiceCache::new(std::env::temp_dir().join(format!("tp-stretch-{}", std::process::id())));
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
    )?;
    compile(
        &program,
        &ScenePlugins::mock(),
        &ctx,
        Path::new("."),
        "0.1.0",
    )
}

fn action_ms(out: &CompileOutput) -> u64 {
    out.timeline.entries[0]
        .action
        .as_ref()
        .unwrap()
        .duration_ms
        .ms()
}

#[test]
fn a_stretched_shot_runs_that_many_times_its_own_length() {
    let plain = compile_it("Go. {#a}\n\n```teleprompt scene=mock\nwait 1000ms\n```\n").unwrap();
    let slow =
        compile_it("Go. {#a}\n\n```teleprompt scene=mock stretch=2\nwait 1000ms\n```\n").unwrap();
    let fast =
        compile_it("Go. {#a}\n\n```teleprompt scene=mock stretch=0.5\nwait 1000ms\n```\n").unwrap();
    assert_eq!(action_ms(&plain), 1000);
    assert_eq!(action_ms(&slow), 2000);
    assert_eq!(action_ms(&fast), 500);
    // Held after its line, the item grows with it.
    let item = |o: &CompileOutput| o.timeline.entries[0].duration_ms;
    assert_eq!(item(&slow) - item(&plain), SpanMs::of(1000));
}

#[test]
fn a_stretch_past_the_bounds_is_refused() {
    let err = compile_it("Go. {#a}\n\n```teleprompt scene=mock stretch=5\nwait 1000ms\n```\n")
        .expect_err("refused");
    assert!(err.0[0].message.contains("outside"), "{err:?}");
}

#[test]
fn stretch_and_a_fitting_policy_are_one_too_many() {
    let err = compile_it(
        "Go. {#a}\n\n```teleprompt scene=mock policy=fit-action stretch=2\nwait 1000ms\n```\n",
    )
    .expect_err("refused");
    assert!(err.0[0].message.contains("both set the pace"), "{err:?}");
}

#[test]
fn stretch_is_a_factor() {
    let err = compile_it("Go. {#a}\n\n```teleprompt scene=mock stretch=-1\nwait 1000ms\n```\n")
        .expect_err("refused");
    assert!(err.0[0].message.contains("factor"), "{err:?}");
}

/// A `fit-action` shot its plugin cannot shorten would be cut off at its
/// slot's end. That is never silent: the warning names the line and says
/// how much is lost.
#[test]
fn a_fit_action_shot_that_cannot_shrink_says_it_is_cut() {
    let out =
        compile_it("Go. {#a}\n\n```teleprompt scene=mock policy=fit-action\nwait 5000ms\n```\n")
            .unwrap();
    assert!(action_ms(&out) < 5000, "the slot is the line's");
    let cut: Vec<_> = out
        .warnings
        .iter()
        .filter(|w| w.contains("cannot shorten"))
        .collect();
    assert_eq!(cut.len(), 1, "{:?}", out.warnings);
    assert!(
        cut[0].contains("#a") || cut[0].contains("a-a#0"),
        "{}",
        cut[0]
    );
}

/// `trim-action` cuts on purpose, and its own warning says so.
#[test]
fn a_trimmed_shot_is_not_said_to_be_cut_by_accident() {
    let out =
        compile_it("Go. {#a}\n\n```teleprompt scene=mock policy=trim-action\nwait 5000ms\n```\n")
            .unwrap();
    assert!(
        !out.warnings.iter().any(|w| w.contains("cannot shorten")),
        "{:?}",
        out.warnings
    );
}
