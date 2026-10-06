use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use teleprompt_core::{DurationSource, SpanMs, TimeMs};

use teleprompt_compile::{compile, CompileOutput, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::check_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Program};
use teleprompt_core::Diagnostics;
use teleprompt_plugin::ScenePlugins;
use teleprompt_voice::cache::VoiceCache;
use teleprompt_voice::NullVoice;
use teleprompt_voice::WpmEstimator;
use teleprompt_voice::{Pcm, VoiceBackend};

fn program(src: &str) -> Program {
    let s = parse_script(src).unwrap();
    resolve(
        &s,
        "demo.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap()
}

/// A cache rooted in a fresh, never-written-to scratch directory, so every
/// call gets a cold cache regardless of what any other test — or an earlier
/// call in the same test — did. `compile` only ever reads its cache, so a
/// directory nothing has created yet reads back as a clean miss on every
/// lookup; the counter just keeps concurrent test threads out of each
/// other's way.
fn throwaway_cache() -> VoiceCache {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    VoiceCache::new(std::env::temp_dir().join(format!(
        "tp-compile-throwaway-{}-{:?}-{n}",
        std::process::id(),
        std::thread::current().id(),
    )))
}

/// Compiles `program` against a fresh cold cache with the reference
/// estimator, returning the raw `Result` so error-path tests can inspect the
/// diagnostics `compile` produced.
fn compile_program(p: &Program) -> Result<CompileOutput, Diagnostics> {
    let cache = throwaway_cache();
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
        takes: &teleprompt_voice::takes::Takes::default(),
    };
    compile(p, &ScenePlugins::mock(), &ctx, Path::new("."), "0.1.0")
}

fn run(src: &str) -> teleprompt_compile::CompileOutput {
    compile_program(&program(src)).unwrap()
}

fn program_for(src: &str) -> Program {
    let parsed = parse_script(src).expect("fixture parses");
    let diags = check_ids(&parsed);
    assert!(
        !diags.iter().any(|d| d.is_error()),
        "fixture must yield unambiguous line ids"
    );
    resolve(
        &parsed,
        "tour.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .expect("fixture resolves")
}

/// Compiles `src` against the caller's own `VoiceContext`, so a test can
/// inspect or warm the cache in between calls.
fn compile_with(src: &str, ctx: &VoiceContext) -> Result<CompileOutput, Diagnostics> {
    let program = program_for(src);
    compile(
        &program,
        &ScenePlugins::mock(),
        ctx,
        Path::new("."),
        "0.1.0",
    )
}

/// A thin wrapper over `compile_with` that builds a throwaway cache, so
/// every test that does not care about cache state keeps working unchanged.
fn compile_str(src: &str) -> Result<CompileOutput, Diagnostics> {
    let cache = throwaway_cache();
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
        takes: &teleprompt_voice::takes::Takes::default(),
    };
    compile_with(src, &ctx)
}

// `transition` lives inside `output:` — a top-level `transition` key fails to
// deserialize under `deny_unknown_fields`.
const ONE_BEAT: &str = r#"---
scene: { mock: { plugin: mock } }
output:
  transition: { duration: 0ms }
---

# Intro

One two three four five six. {#welcome}

```teleprompt scene=mock
wait 500ms
```
"#;

#[test]
fn narration_and_the_following_action_form_one_shot() {
    let out = run(ONE_BEAT);
    assert_eq!(out.timeline.entries.len(), 1);
    let e = &out.timeline.entries[0];
    assert!(e.narration.is_some());
    assert!(e.action.is_some());
}

/// On a cold cache the duration is the estimator's prediction — no backend
/// is involved, and `compile` has no way to reach one.
#[test]
fn a_cold_narration_duration_comes_from_the_estimator() {
    let out = run(ONE_BEAT);
    let n = out.timeline.entries[0].narration.as_ref().unwrap();
    // 6 words at 150 wpm = 2400ms, plus 350ms for the full stop
    assert_eq!(n.duration_ms, SpanMs::of(2750));
    assert_eq!(n.duration_source, DurationSource::Estimated);
}

#[test]
fn action_duration_comes_from_the_scene_estimate() {
    let out = run(ONE_BEAT);
    assert_eq!(
        out.timeline.entries[0].action.as_ref().unwrap().duration_ms,
        SpanMs::of(500)
    );
    assert_eq!(
        out.timeline.entries[0]
            .action
            .as_ref()
            .unwrap()
            .duration_source,
        DurationSource::Exact
    );
}

#[test]
fn each_mark_after_the_first_shot_becomes_its_own_shot() {
    let src = r#"---
scene: { mock: { plugin: mock } }
---

# Intro

One. {#a}

```teleprompt scene=mock
wait 100ms
mark
wait 200ms
mark
wait 300ms
```
"#;
    let out = run(src);
    assert_eq!(out.timeline.entries.len(), 3);
    assert!(out.timeline.entries[0].narration.is_some());
    assert!(out.timeline.entries[1].narration.is_none());
    assert_eq!(
        out.timeline.entries[1].action.as_ref().unwrap().duration_ms,
        SpanMs::of(200)
    );
    assert_eq!(
        out.timeline.entries[2].action.as_ref().unwrap().duration_ms,
        SpanMs::of(300)
    );
}

#[test]
fn a_pause_directive_becomes_a_silent_shot() {
    let src = "---\nscene: { mock: { plugin: mock } }\n---\n\n# A\n\nOne. {#a}\n\n<!-- teleprompt: pause 800ms -->\n";
    let out = run(src);
    assert_eq!(out.timeline.entries.len(), 2);
    let p = &out.timeline.entries[1];
    assert_eq!(p.duration_ms, SpanMs::of(800));
    assert!(p.narration.is_none(), "a pause is silent");
    assert_eq!(p.action.as_ref().unwrap().scene, "pause");
}

#[test]
fn the_scene_records_both_its_name_and_its_plugin() {
    let out = run(ONE_BEAT);
    let a = out.timeline.entries[0].action.as_ref().unwrap();
    assert_eq!(a.scene, "mock");
    assert_eq!(a.plugin, "mock");
}

/// A scene plugin's name is a scene without declaring one.
#[test]
fn an_undeclared_scene_named_after_an_plugin_uses_it() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=mock\nwait 100ms\n```\n";
    let out = run(src);
    let action = out.timeline.entries[0].action.as_ref().unwrap();
    assert_eq!(
        (action.scene.as_str(), action.plugin.as_str()),
        ("mock", "mock")
    );
}

/// A name that is neither declared nor a scene plugin is a mistake, not a
/// placeholder video.
#[test]
fn an_unknown_scene_is_an_error_naming_the_plugins() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=moc\nwait 100ms\n```\n";
    let e = compile_program(&program(src)).unwrap_err();
    assert!(
        e.0[0].message.contains("unknown scene `moc`"),
        "{:?}",
        e.0[0]
    );
    let help = e.0[0].help.as_deref().unwrap();
    assert!(
        help.contains("[scene.moc]") && help.contains("mock"),
        "{help}"
    );
}

#[test]
fn an_unavailable_scene_plugin_is_a_diagnostic_not_a_panic() {
    let src = "---\nscene:\n  web:\n    plugin: playwright\n---\n\n# A\n\nOne. {#a}\n\n```teleprompt scene=web\nawait page.goto('/');\n```\n";
    let p = program(src);
    let e = compile_program(&p).unwrap_err();
    let message = &e.0[0].message;
    assert!(
        message.contains("scene plugin `playwright`, which is not available"),
        "{message}"
    );
    assert!(e.0[0].help.as_deref().unwrap().contains("mock"));
}

#[test]
fn plugin_validation_errors_reach_the_caller() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=mock\nclick everything\n```\n";
    let p = program(src);
    let e = compile_program(&p).unwrap_err();
    assert!(e.0[0].message.contains("unknown mock directive"));
}

/// Reported where the block was written, before compiling: the policy is
/// read with the block's other attributes.
#[test]
fn an_invalid_policy_is_a_diagnostic_at_its_block() {
    let src = "# A\n\nOne. {#a}\n\n```teleprompt scene=mock policy=sideways\nwait 1s\n```\n";
    let s = parse_script(src).unwrap();
    let e = resolve(
        &s,
        "demo.md",
        "en",
        &PartialConfig::default(),
        &PartialConfig::default(),
    )
    .unwrap_err();
    assert!(e.0[0].message.contains("unknown policy `sideways`"));
    assert_eq!(e.0[0].span.map(|s| s.line), Some(5));
}

#[test]
fn compilation_is_deterministic() {
    let a = run(ONE_BEAT);
    let b = run(ONE_BEAT);
    assert_eq!(
        serde_json::to_string(&a.timeline).unwrap(),
        serde_json::to_string(&b.timeline).unwrap()
    );
}

// `Element::Action`'s `shot` must be threaded into `BlockSource` for real, not
// `SourceSpan { line: 0, .. }`. The fence here opens on line 14 (verified
// against `parse_script` independently), so the invalid `bogus` directive on
// the block's second body line sits at absolute source line 16 (14 + 1 + 1, per
// the mock plugin's `shot.line + i + 1` convention). A fabricated `line: 0`
// would instead report line 2 (0 + 1 + 1) — a small, visibly wrong number that
// a real user's editor would never scroll to.
const SPAN_SRC: &str = "# A\n\nOne. {#a}\n\n\n\n\n\n\n\n\n\n\n```teleprompt scene=mock\nwait 100ms\nbogus directive\n```\n";

#[test]
fn plugin_diagnostics_report_the_real_source_line_not_a_fabricated_zero() {
    let p = program(SPAN_SRC);
    let e = compile_program(&p).unwrap_err();
    assert!(e.0[0].message.contains("unknown mock directive `bogus`"));
    assert_eq!(
        e.0[0].span.unwrap().line,
        16,
        "must be the real absolute line, not the small number a fabricated \
         `SourceSpan {{ line: 0, .. }}` would have produced"
    );
}

// Dropping empty shots can reduce an all-`mark` action block to zero surviving
// shots. A narration must not reach across that empty block to pair with a
// later one — pairing stays scoped to the immediately following action item.
#[test]
fn a_narration_does_not_jump_an_empty_action_block_to_pair_with_a_later_one() {
    let src = r#"---
scene: { mock: { plugin: mock } }
---

# Intro

One. {#a}

```teleprompt scene=mock
mark
```

```teleprompt scene=mock
wait 400ms
```
"#;
    let out = run(src);
    assert_eq!(out.timeline.entries.len(), 2);
    assert!(
        out.timeline.entries[0].narration.is_some(),
        "the narration becomes its own item"
    );
    assert!(
        out.timeline.entries[0].action.is_none(),
        "the empty block produces no item, so nothing attaches to the narration"
    );
    assert!(
        out.timeline.entries[1].narration.is_none(),
        "the narration must not jump the empty block to reach the second one"
    );
    assert_eq!(
        out.timeline.entries[1].action.as_ref().unwrap().duration_ms,
        SpanMs::of(400)
    );
}

// Regression pin: a narration followed by an all-`mark` block at the end of
// the program must still flush as a narration-only item (this already
// worked before the fix above; pinned so it stays that way).
#[test]
fn a_narration_followed_by_an_empty_action_block_at_end_of_program_still_flushes() {
    let src = r#"---
scene: { mock: { plugin: mock } }
---

# Intro

One. {#a}

```teleprompt scene=mock
mark
```
"#;
    let out = run(src);
    assert_eq!(out.timeline.entries.len(), 1);
    assert!(out.timeline.entries[0].narration.is_some());
    assert!(out.timeline.entries[0].action.is_none());
}

// A `Item` carries one `Config`, but its two halves resolve from different
// layers — the narration's from the line's own attributes, the item's from the
// following action block's. `compile` used to set `config: config.clone()` (the
// action block's) and read the narration's padding off that, so an identical
// line produced different narration timing depending on whether an action block
// happened to follow it. A line is usually followed by an action block, so the
// broken case was the normal one.
const SEGMENT_WITH_LEAD_IN: &str = "One two three. {#a lead_in=1000ms}";

fn lead_in_fixture(with_action: bool) -> String {
    let mut s = format!(
        "---\nscene: {{ mock: {{ plugin: mock }} }}\n---\n\n# A\n\n{SEGMENT_WITH_LEAD_IN}\n"
    );
    if with_action {
        s.push_str("\n```teleprompt scene=mock\nwait 100ms\n```\n");
    }
    s
}

#[test]
fn a_lines_lead_in_survives_pairing_with_a_following_action_block() {
    let alone = run(&lead_in_fixture(false));
    let paired = run(&lead_in_fixture(true));

    let alone_n = alone.timeline.entries[0].narration.as_ref().unwrap();
    let paired_n = paired.timeline.entries[0].narration.as_ref().unwrap();

    assert_eq!(
        alone_n.start_ms,
        TimeMs::at(1000),
        "the line's own lead_in=1000ms places its narration"
    );
    assert_eq!(
        paired_n.start_ms, alone_n.start_ms,
        "the same line must produce the same narration start whether or \
         not an action block follows"
    );

    // The item's own duration differs by exactly the action block's 100ms
    // under `hold`, and by nothing else: the 850ms that used to vanish with
    // the discarded lead-in is gone.
    assert_eq!(alone.timeline.entries[0].duration_ms, SpanMs::of(2700));
    assert_eq!(paired.timeline.entries[0].duration_ms, SpanMs::of(2800));
}

#[test]
fn compile_retains_the_text_and_chapter_the_timeline_drops() {
    let src = "\
# Quick start

Every video here is built from a script.

# Provenance

And every timeline is committed.
";
    let out = compile_str(src).expect("compiles");

    let ids: Vec<&str> = out.narration.iter().map(|n| n.line_id.as_str()).collect();
    assert_eq!(
        ids.len(),
        2,
        "one detail per narration item, in document order"
    );

    assert_eq!(
        out.narration[0].text,
        "Every video here is built from a script."
    );
    assert_eq!(out.narration[0].chapter, "quick-start");
    assert_eq!(out.narration[1].chapter, "provenance");

    assert!(
        out.narration[0].word_timings.is_none(),
        "the null backend advertises word_timings: false"
    );
}

#[test]
fn compile_carries_the_scripts_chapters() {
    let out = compile_str(
        "\
# Quick start

The first paragraph.

# Provenance

The second paragraph.
",
    )
    .expect("compiles");

    let slugs: Vec<&str> = out.chapters.iter().map(|c| c.slug.as_str()).collect();
    assert_eq!(
        slugs,
        vec!["quick-start", "provenance"],
        "the CLI reaches compilation only through `Script::compile`, which \
         returns this struct — chapters unreachable here are unreachable to `dub`"
    );
}

#[test]
fn narration_details_line_up_with_timeline_narration_entries() {
    let src = "\
# Quick start

The first paragraph here.

The second paragraph here.
";
    let out = compile_str(src).expect("compiles");

    let timeline_ids: Vec<&str> = out
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref().map(|n| n.line.as_str()))
        .collect();
    let detail_ids: Vec<&str> = out.narration.iter().map(|n| n.line_id.as_str()).collect();

    assert_eq!(
        timeline_ids, detail_ids,
        "the join key must be total in both directions, or the manifest \
         will silently drop or invent lines"
    );
}

/// The request `compile` measured the duration from is the request it hands
/// out, so a caller that renders audio cannot render something else. `dub`
/// used to rebuild a `SynthRequest` itself, dropping `voice` and `speed`,
/// and published a duration from the resolved config beside a file rendered
/// at the defaults.
#[tokio::test]
async fn the_narration_detail_carries_the_resolved_synth_request() {
    let src = "\
---
voice:
  voice: narrator
  speed: 2.0
---

# A

One two three four five six.
";
    let out = compile_str(src).expect("compiles");
    let detail = &out.narration[0];

    assert_eq!(detail.synth_request.text, detail.text);
    assert_eq!(detail.synth_request.locale, "en");
    assert_eq!(detail.synth_request.voice.as_deref(), Some("narrator"));
    assert_eq!(
        detail.synth_request.speed, 2.0,
        "the resolved config's speed, not the default"
    );

    // And it is the same request the published duration was measured from.
    let measured = NullVoice::default()
        .synthesize(&detail.synth_request)
        .await
        .unwrap();
    let published = out.timeline.entries[0]
        .narration
        .as_ref()
        .unwrap()
        .duration_ms;
    assert_eq!(measured.pcm.duration_ms(), published.ms());
}

#[test]
fn the_narration_detail_carries_the_chapter_index_as_well_as_the_slug() {
    let src = "\
# Setup

First. {#one}

# Setup

Second. {#two}
";
    let out = compile_str(src).expect("compiles");
    let by_index: Vec<(usize, &str)> = out
        .narration
        .iter()
        .map(|d| (d.chapter_index, d.chapter.as_str()))
        .collect();
    assert_eq!(
        by_index,
        vec![(0, "setup"), (1, "setup")],
        "the slug cannot tell two identically-titled chapters apart; the \
         index must"
    );
}

fn cache_dir(tag: &str) -> teleprompt_testkit::TestDir {
    teleprompt_testkit::test_dir(&format!("compile-{tag}"))
}

const ONE: &str = "# Quick start\n\nOne two three four five six.\n";

#[test]
fn a_cold_cache_yields_estimated_durations() {
    let dir = cache_dir("cold");
    let cache = VoiceCache::new(dir.path());
    let est = WpmEstimator::default();
    let ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &est,
        takes: &teleprompt_voice::takes::Takes::default(),
    };
    let out = compile_with(ONE, &ctx).expect("compiles");

    let n = out.timeline.entries[0].narration.as_ref().unwrap();
    assert_eq!(n.duration_source, DurationSource::Estimated);
    // 6 words at 150 wpm = 2400ms speech, plus 350ms for `ONE`'s trailing
    // full stop — the same shared `estimate_ms` model `NullVoice` and
    // `WpmEstimator` both call, and the same total
    // `a_cold_narration_duration_comes_from_the_estimator` pins for this exact
    // sentence.
    assert_eq!(
        n.duration_ms,
        SpanMs::of(2750),
        "six words at 150 wpm, plus the full stop"
    );
}

#[test]
fn a_warm_cache_yields_measured_durations() {
    let dir = cache_dir("warm");
    let cache = VoiceCache::new(dir.path());
    let est = WpmEstimator::default();
    let ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &est,
        takes: &teleprompt_voice::takes::Takes::default(),
    };

    // Populate the cache under the key compile will look up, with audio of a
    // deliberately different length from the estimate.
    let cold = compile_with(ONE, &ctx).expect("compiles");
    let k = cold.narration[0].cache_key.clone();
    cache
        .store(
            &k,
            &Pcm {
                sample_rate: 24_000,
                channels: 1,
                samples: vec![0; 24_000 * 5],
            },
            None,
        )
        .unwrap();

    let warm = compile_with(ONE, &ctx).expect("compiles");
    let n = warm.timeline.entries[0].narration.as_ref().unwrap();
    assert_eq!(n.duration_source, DurationSource::Measured);
    assert_eq!(
        n.duration_ms,
        SpanMs::of(5000),
        "the cached audio's real length, not the estimate"
    );
}

#[test]
fn the_cache_key_covers_the_resolved_voice_config() {
    let dir = cache_dir("cfgkey");
    let cache = VoiceCache::new(dir.path());
    let est = WpmEstimator::default();
    let ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &est,
        takes: &teleprompt_voice::takes::Takes::default(),
    };

    let plain = compile_with(ONE, &ctx).unwrap();
    let fast = compile_with(
        "---\nvoice: { speed: 2.0 }\n---\n\n# Quick start\n\nOne two three four five six.\n",
        &ctx,
    )
    .unwrap();

    assert_ne!(
        plain.narration[0].cache_key.to_string(),
        fast.narration[0].cache_key.to_string(),
        "a different speed is different audio and must not share a cache entry"
    );
}

/// Pins the reasoning behind `Project::compile` reading `backend_id` off the
/// *resolved* config (`program.config.voice.backend`) rather than the
/// project's unresolved default: the cache key has to cover whichever
/// backend actually produces a line's audio, or two backends could
/// collide on one cache entry.
#[test]
fn the_cache_key_covers_the_backend_id() {
    let dir = cache_dir("backendkey");
    let cache = VoiceCache::new(dir.path());
    let est = WpmEstimator::default();
    let null_ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &est,
        takes: &teleprompt_voice::takes::Takes::default(),
    };
    let other_ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "other",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &est,
        takes: &teleprompt_voice::takes::Takes::default(),
    };

    let null = compile_with(ONE, &null_ctx).unwrap();
    // The line's own backend decides: the script asks for `other`.
    let other_script = format!("---\nvoice: {{ backend: other }}\n---\n\n{ONE}");
    let other = compile_with(&other_script, &other_ctx).unwrap();

    assert_ne!(
        null.narration[0].cache_key.to_string(),
        other.narration[0].cache_key.to_string(),
        "the same text under two different backends is different audio and must \
         not share a cache entry"
    );
}

/// Every `narration.duration_source` in a serialized timeline replaced by
/// one fixed token.
///
/// Comparing the serialized form rather than field by field is deliberate:
/// a field added to `NarrationEntry` or `TimelineEntry` tomorrow is compared
/// automatically, whereas a hand-written comparison would silently stop
/// covering it. Only the narration's `duration_source` is normalized — an
/// *action*'s must still match, or a cache hit quietly changing one would
/// slip through.
fn timeline_modulo_duration_source(t: &teleprompt_schedule::Timeline) -> serde_json::Value {
    let mut v = serde_json::to_value(t).expect("a timeline always serializes");
    for entry in v["entries"].as_array_mut().expect("entries is an array") {
        if let Some(source) = entry.pointer_mut("/narration/duration_source") {
            *source = serde_json::Value::String("<normalized>".to_string());
        }
    }
    v
}

/// A cache hit and a cache miss produce identical timelines apart from
/// `duration_source`.
///
/// `a_warm_cache_yields_measured_durations` deliberately caches audio of a
/// different length, which proves the duration is read from the cache but
/// says nothing about this identity claim. Here the cached audio is exactly
/// what the backend would have rendered — the null backend's own output for
/// the very request `compile` measured — so everything except the one field
/// must survive the round trip untouched.
#[tokio::test]
async fn a_cache_hit_and_a_cache_miss_agree_on_everything_but_duration_source() {
    let dir = cache_dir("roundtrip");
    let cache = VoiceCache::new(dir.path());
    let est = WpmEstimator::default();
    let ctx = VoiceContext {
        other_backends: Default::default(),
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &est,
        takes: &teleprompt_voice::takes::Takes::default(),
    };

    let cold = compile_with(ONE, &ctx).expect("compiles");
    assert_eq!(
        cold.timeline.entries[0]
            .narration
            .as_ref()
            .unwrap()
            .duration_source,
        DurationSource::Estimated
    );

    // The audio the backend really would have produced for the request
    // `compile` measured, stored under the key `compile` looked up.
    let detail = &cold.narration[0];
    let rendered = NullVoice::default()
        .synthesize(&detail.synth_request)
        .await
        .expect("null synthesizes");
    cache
        .store(&detail.cache_key, &rendered.pcm, None)
        .expect("stores");

    let warm = compile_with(ONE, &ctx).expect("compiles");
    assert_eq!(
        warm.timeline.entries[0]
            .narration
            .as_ref()
            .unwrap()
            .duration_source,
        DurationSource::Measured
    );

    // The field really did differ, so the normalization below is hiding a
    // real difference rather than papering over an identical pair.
    assert_ne!(
        serde_json::to_value(&cold.timeline).unwrap(),
        serde_json::to_value(&warm.timeline).unwrap()
    );

    assert_eq!(
        timeline_modulo_duration_source(&cold.timeline),
        timeline_modulo_duration_source(&warm.timeline),
        "a cache hit must change which numbers are measurements, and nothing else"
    );
}

/// The pronunciation map reaches the synthesizer and nothing else. What
/// gets spoken is the mapped form; what gets published — the manifest's
/// text, a caption, the script itself — is what the author wrote, because
/// "em-double-you-ay-ay" is not a word anyone wants to read.
#[test]
fn a_pronunciation_changes_what_is_spoken_and_not_what_is_published() {
    let src = "---\nteleprompt: 1\nvoice:\n  pronounce:\n    MWAA: em-double-you-ay-ay\n---\n\n\
               # Services\n\nIt works against MWAA today. {#services}\n";
    let out = compile_program(&program(src)).expect("compiles");

    let detail = &out.narration[0];
    assert_eq!(
        detail.text, "It works against MWAA today.",
        "the published text keeps the spelling"
    );
    assert_eq!(
        detail.synth_request.text, "It works against em-double-you-ay-ay today.",
        "and the voice is given the pronunciation"
    );
}

/// Changing how a word is said is changing the audio, so it has to change
/// the key that audio is stored under. Otherwise the first render of a
/// script wins forever and the fix is inaudible.
#[test]
fn changing_a_pronunciation_invalidates_the_cached_audio() {
    let plain = compile_program(&program(
        "---\nteleprompt: 1\n---\n\n# S\n\nIt works against MWAA today. {#services}\n",
    ))
    .expect("compiles");
    let said = compile_program(&program(
        "---\nteleprompt: 1\nvoice:\n  pronounce:\n    MWAA: em-double-you-ay-ay\n---\n\n\
         # S\n\nIt works against MWAA today. {#services}\n",
    ))
    .expect("compiles");

    assert_ne!(
        plain.narration[0].cache_key, said.narration[0].cache_key,
        "the same text said differently is different audio"
    );
}

/// The complaint that produced this: the terminal types a command while
/// the voice is somewhere else in the paragraph. `at=` anchors the action
/// to the words that name it.
#[test]
fn an_action_cued_to_a_phrase_starts_when_that_phrase_is_spoken() {
    let src = "# Config\n\n\
               One command registers a server. `flowrs config add` asks for a name. {#c}\n\n\
               ```teleprompt scene=mock policy=concurrent cue=\"config add\"\n\
               wait 500ms\n\
               ```\n";
    let out = compile_program(&program(src)).expect("compiles");

    let entry = &out.timeline.entries[0];
    let narration = entry.narration.as_ref().expect("the item is narrated");
    let action = entry.action.as_ref().expect("and has an action");

    // The phrase sits a little over halfway through the sentence, so the
    // action starts a little over halfway through the speech rather than
    // at its first word.
    let fraction =
        (action.start_ms - narration.start_ms).ms() as f64 / narration.duration_ms.ms() as f64;
    assert!(
        (0.4..0.8).contains(&fraction),
        "cued {fraction:.2} of the way through, not near the phrase"
    );
}

/// Where the phrase sits is measured in characters: Greek letters are two
/// bytes each, and counting bytes put this cue past the end of the speech.
#[test]
fn a_cue_after_non_ascii_text_starts_where_the_phrase_is_spoken() {
    let src = "# Config\n\n\
               Καλημέρα κόσμε, καλησπέρα κόσμε, and then the cue. {#c}\n\n\
               ```teleprompt scene=mock policy=concurrent cue=\"the cue\"\n\
               wait 500ms\n\
               ```\n";
    let out = compile_program(&program(src)).expect("compiles");

    let entry = &out.timeline.entries[0];
    let narration = entry.narration.as_ref().expect("the item is narrated");
    let action = entry.action.as_ref().expect("and has an action");
    let fraction =
        (action.start_ms - narration.start_ms).ms() as f64 / narration.duration_ms.ms() as f64;
    assert!(
        (0.6..0.95).contains(&fraction),
        "cued {fraction:.2} of the way through, not near the phrase"
    );
}

/// A shot that names something the paragraph does not say is a typo, and a
/// typo that silently does nothing is the kind that ships.
#[test]
fn a_cue_that_is_not_in_the_narration_is_an_error() {
    let src = "# Config\n\nOne command registers a server. {#c}\n\n\
               ```teleprompt scene=mock policy=concurrent cue=\"config add\"\n\
               wait 500ms\n\
               ```\n";
    let errors = compile_program(&program(src)).expect_err("a shot must be findable");
    let rendered = format!("{errors:?}");
    assert!(rendered.contains("config add"), "{rendered}");
}

/// `hold` runs the action after the narration and the stretch policies size
/// it to fit; a shot would contradict either rather than refine it.
#[test]
fn a_cue_only_makes_sense_where_the_two_run_together() {
    let src = "# Config\n\nOne command registers a server. {#c}\n\n\
               ```teleprompt scene=mock policy=hold cue=\"command\"\n\
               wait 500ms\n\
               ```\n";
    let errors = compile_program(&program(src)).expect_err("a shot needs concurrent");
    let rendered = format!("{errors:?}");
    assert!(rendered.contains("concurrent"), "{rendered}");
}

/// Declaring a scene with nothing in it changes nothing, so it captures to
/// the same clips as not declaring it.
#[test]
fn a_scene_declared_empty_keys_as_one_left_undeclared() {
    let body = "# A\n\nOne. {#a}\n\n```teleprompt scene=mock\nwait 100ms\n```\n";
    let declared = format!("---\nscene:\n  mock:\n    plugin: mock\n---\n\n{body}");
    let key = |src: &str| {
        run(src).timeline.entries[0]
            .action
            .as_ref()
            .unwrap()
            .capture_key
    };
    assert_eq!(key(body), key(&declared));
}
