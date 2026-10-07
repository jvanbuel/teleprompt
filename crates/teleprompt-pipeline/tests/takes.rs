//! A recorded take stands in for a line's synthesized voice while the line
//! still reads as it did when recorded.

use std::path::Path;

use teleprompt_core::{DurationSource, SpanMs};
use teleprompt_pipeline::compile::{compile, CompileOutput, VoiceContext};
use teleprompt_pipeline::schedule::NarrationEntry;
use teleprompt_scene::ScenePlugins;
use teleprompt_script::config::PartialConfig;
use teleprompt_script::parse::parse_script;
use teleprompt_script::program::resolve;
use teleprompt_voice::cache::VoiceCache;
use teleprompt_voice::takes::Takes;
use teleprompt_voice::Pcm;
use teleprompt_voice::WpmEstimator;

const SRC: &str = "# Tour\n\nWelcome to Acme. {#welcome}\n\nDeployment is one command. {#deploy}\n";

fn pcm(ms: usize, tone: i16) -> Pcm {
    Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![tone; ms * 48],
    }
}

fn compiled(takes: &Takes) -> CompileOutput {
    let script = parse_script(SRC).unwrap();
    let program = resolve(
        &script,
        "demo.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap();
    let dir = teleprompt_testkit::test_dir("compile-takes-cache");
    let cache = VoiceCache::new(dir.join("cache"));
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
        takes,
    };
    compile(
        &program,
        &ScenePlugins::mock(),
        &ctx,
        Path::new("."),
        "0.1.0",
    )
    .unwrap()
}

fn line<'a>(out: &'a CompileOutput, id: &str) -> &'a NarrationEntry {
    out.timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .find(|n| n.line == id)
        .unwrap()
}

#[test]
fn a_current_take_is_its_lines_voice_and_length() {
    let dir = teleprompt_testkit::test_dir("compile-takes-current");
    let mut takes = Takes::load(&dir.join("takes")).unwrap();
    takes
        .save("welcome", "Welcome to Acme.", &pcm(2345, 7))
        .unwrap();

    let out = compiled(&takes);
    let welcome = line(&out, "welcome");
    assert_eq!(welcome.duration_ms, SpanMs::of(2345));
    assert_eq!(welcome.duration_source, DurationSource::Measured);
    assert!(welcome.recorded);
    let deploy = line(&out, "deploy");
    assert!(!deploy.recorded, "a line with no take is synthesized");
    assert_eq!(deploy.duration_source, DurationSource::Estimated);

    let detail = out
        .narration
        .iter()
        .find(|d| d.line_id == "welcome")
        .unwrap();
    assert_eq!(detail.take.as_ref().map(|t| t.duration_ms), Some(2345));
    assert!(out
        .narration
        .iter()
        .find(|d| d.line_id == "deploy")
        .unwrap()
        .take
        .is_none());
}

/// A take of the line as it used to read is not a take of it now.
#[test]
fn a_take_of_an_edited_line_is_left_out() {
    let dir = teleprompt_testkit::test_dir("compile-takes-stale");
    let mut takes = Takes::load(&dir.join("takes")).unwrap();
    takes
        .save("welcome", "Welcome to Acme Corp.", &pcm(2345, 7))
        .unwrap();

    let welcome = line(&compiled(&takes), "welcome").clone();
    assert!(!welcome.recorded);
    assert_eq!(welcome.duration_source, DurationSource::Estimated);
}

/// Recording a line again changes its audio, as the timeline records it, so
/// `plan --check` sees the new take even at the same length.
#[test]
fn a_new_take_changes_the_lines_audio() {
    let dir = teleprompt_testkit::test_dir("compile-takes-again");
    let mut takes = Takes::load(&dir.join("takes")).unwrap();
    takes
        .save("welcome", "Welcome to Acme.", &pcm(2345, 7))
        .unwrap();
    let first = line(&compiled(&takes), "welcome").audio_hash;
    takes
        .save("welcome", "Welcome to Acme.", &pcm(2345, 9))
        .unwrap();
    let second = line(&compiled(&takes), "welcome").audio_hash;
    assert_ne!(first, second);
}
