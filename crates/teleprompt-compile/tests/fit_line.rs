//! `fit-line`: the picture leads, and its line is sped up or slowed down
//! to fit it (docs/design.md#led-by-the-picture).

use std::path::Path;

use teleprompt_compile::{compile, CompileOutput, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::resolve;
use teleprompt_core::{Diagnostics, PolicyKind, SpanMs, Tempo};
use teleprompt_plugin::ScenePlugins;
use teleprompt_voice::cache::VoiceCache;
use teleprompt_voice::WpmEstimator;

const LINE: &str = "Deployment is one command, and it streams progress as it goes. {#deploy}";

fn compile_it(block: &str) -> Result<CompileOutput, Diagnostics> {
    let src = format!("# A\n\n{LINE}\n\n```teleprompt scene=mock {block}\n```\n");
    let cache =
        VoiceCache::new(std::env::temp_dir().join(format!("tp-fit-line-{}", std::process::id())));
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

fn errors(block: &str) -> Vec<String> {
    compile_it(block)
        .err()
        .map(|d| d.0.iter().map(|d| d.message.clone()).collect())
        .unwrap_or_default()
}

/// The line's clip as the voice says it, at its own pace.
fn clip_ms() -> u64 {
    let out = compile_it("policy=concurrent\nwait 100ms").unwrap();
    out.timeline.entries[0]
        .narration
        .as_ref()
        .unwrap()
        .duration_ms
        .ms()
}

/// A picture that leaves the clip `ms`, between the default 150 ms lead-in
/// and tail.
fn picture(ms: u64) -> String {
    format!("policy=fit-line\nwait {}ms", ms + 300)
}

#[test]
fn a_line_is_sped_up_to_fit_its_picture() {
    let clip = clip_ms();
    let fits = clip * 10 / 11;
    let out = compile_it(&picture(fits)).unwrap();
    let entry = &out.timeline.entries[0];
    let n = entry.narration.as_ref().unwrap();
    assert_eq!(entry.policy, PolicyKind::FitLine);
    assert_eq!(n.tempo_permille, Tempo::new(1100));
    assert!(
        n.duration_ms.ms().abs_diff(fits) <= 1,
        "{} vs {fits}",
        n.duration_ms.ms()
    );
    // The picture keeps its length, and so is the item's.
    assert_eq!(entry.duration_ms, SpanMs::of(fits + 300));
    assert_eq!(
        entry.action.as_ref().unwrap().duration_ms,
        SpanMs::of(fits + 300)
    );
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn a_line_too_long_for_its_picture_stops_at_the_bound_and_says_what_to_cut() {
    let clip = clip_ms();
    let out = compile_it(&picture(clip / 2)).unwrap();
    let n = out.timeline.entries[0].narration.as_ref().unwrap();
    assert_eq!(n.tempo_permille, Tempo::new(1150));
    let w = out.warnings.join("\n");
    assert!(w.contains("deploy") && w.contains("max_line_speed"), "{w}");
    // Twice too long at 1.15x: about 5 of its 11 words to go.
    assert!(w.contains("cut about 5 of its 11 words"), "{w}");
}

#[test]
fn a_line_short_for_its_picture_slows_to_the_bound_and_the_picture_plays_on() {
    let clip = clip_ms();
    let out = compile_it(&picture(clip * 2)).unwrap();
    let entry = &out.timeline.entries[0];
    assert_eq!(
        entry.narration.as_ref().unwrap().tempo_permille,
        Tempo::new(900)
    );
    assert_eq!(entry.duration_ms, SpanMs::of(clip * 2 + 300));
    assert!(
        out.warnings.join("\n").contains("min_line_speed"),
        "{:?}",
        out.warnings
    );
}

#[test]
fn a_line_that_fits_as_it_is_keeps_its_tempo() {
    let out = compile_it(&picture(clip_ms())).unwrap();
    assert_eq!(
        out.timeline.entries[0]
            .narration
            .as_ref()
            .unwrap()
            .tempo_permille,
        None
    );
    let other = compile_it("policy=concurrent\nwait 100ms").unwrap();
    assert_eq!(
        other.timeline.entries[0]
            .narration
            .as_ref()
            .unwrap()
            .tempo_permille,
        None
    );
}

#[test]
fn a_budget_is_the_length_of_a_picture_that_states_none() {
    // Long enough for the line at its slowest: the picture sets the length.
    let budget = clip_ms() * 2;
    let out = compile_it(&format!("policy=fit-line budget={budget}ms\nopen")).unwrap();
    assert_eq!(out.timeline.entries[0].duration_ms, SpanMs::of(budget));
}

#[test]
fn fit_line_needs_a_length_and_takes_no_budget_where_the_shot_has_one() {
    let none = errors("policy=fit-line\nopen");
    assert!(
        none.iter().any(|e| e.contains("states no length")),
        "{none:?}"
    );
    let both = errors("policy=fit-line budget=4s\nwait 2s");
    assert!(
        both.iter().any(|e| e.contains("its own length")),
        "{both:?}"
    );
    let cued = errors("policy=fit-line cue=\"streams\"\nwait 2s");
    assert!(cued.iter().any(|e| e.contains("cue")), "{cued:?}");
}
