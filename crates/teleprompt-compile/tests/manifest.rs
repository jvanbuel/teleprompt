use std::path::Path;

use teleprompt_cache::VoiceCache;
use teleprompt_compile::manifest::{self, AudioInfo, MANIFEST_VERSION};
use teleprompt_compile::{compile, CompileOutput, VoiceContext};
use teleprompt_core::config::PartialConfig;
use teleprompt_core::ident::assign_ids;
use teleprompt_core::parse::parse_script;
use teleprompt_core::program::{resolve, Program};
use teleprompt_scene::SceneRegistry;
use teleprompt_voice::{Pcm, WordTiming};
use teleprompt_voice_null::WpmEstimator;

fn program_for(src: &str) -> Program {
    let mut parsed = parse_script(src).expect("fixture parses");
    let diags = assign_ids(&mut parsed);
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

/// A cache rooted in a fresh scratch directory, so every call gets a cold
/// cache regardless of what any other test did.
fn throwaway_cache() -> VoiceCache {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    VoiceCache::new(std::env::temp_dir().join(format!(
        "tp-manifest-throwaway-{}-{:?}-{n}",
        std::process::id(),
        std::thread::current().id(),
    )))
}

fn compiled_and_manifest(src: &str) -> (CompileOutput, manifest::NarrationManifest) {
    let program = program_for(src);
    let cache = throwaway_cache();
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
    };
    let out = compile(
        &program,
        &SceneRegistry::with_builtins(),
        &ctx,
        Path::new("."),
        "0.1.0",
    )
    .unwrap();
    let m = manifest::build(
        &out.timeline,
        &program.chapters,
        &out.narration,
        AudioInfo {
            format: "wav".to_string(),
            sample_rate: 48_000,
            channels: 1,
        },
    );
    (out, m)
}

fn manifest_for(src: &str) -> manifest::NarrationManifest {
    compiled_and_manifest(src).1
}

/// Builds the manifest from a *warm* cache carrying `word_timings` for every
/// line, so the manifest's `WordTiming.word` -> `WordEntry.text` rename
/// has something to actually exercise. `compile` never synthesizes, so word
/// timings only ever reach it through a cache hit — there is no longer a
/// backend to stub for this.
fn manifest_for_with_words(
    src: &str,
    word_timings: Vec<WordTiming>,
) -> manifest::NarrationManifest {
    let program = program_for(src);
    let cache = throwaway_cache();
    let estimator = WpmEstimator::default();
    let ctx = VoiceContext {
        backend_id: "null",
        backend_version: "0.1.0",
        cache: &cache,
        estimator: &estimator,
    };

    let cold = compile(
        &program,
        &SceneRegistry::with_builtins(),
        &ctx,
        Path::new("."),
        "0.1.0",
    )
    .unwrap();
    for detail in &cold.narration {
        cache
            .store(
                &detail.cache_key,
                &Pcm {
                    sample_rate: 48_000,
                    channels: 1,
                    samples: vec![0; 48_000],
                },
                Some(&word_timings),
            )
            .unwrap();
    }

    let warm = compile(
        &program,
        &SceneRegistry::with_builtins(),
        &ctx,
        Path::new("."),
        "0.1.0",
    )
    .unwrap();
    manifest::build(
        &warm.timeline,
        &program.chapters,
        &warm.narration,
        AudioInfo {
            format: "wav".to_string(),
            sample_rate: 48_000,
            channels: 1,
        },
    )
}

const ONE_BLOCK_SPLIT_BY_A_MARK: &str = "\
# Quick start

Every video in this repository is built from a script you can read. {#intro}

```teleprompt scene=mock policy=concurrent
wait 500ms
mark
wait 200ms
```
";

const TWO_CHAPTERS: &str = "\
# Quick start

Every video in this repository is built from a script you can read.

# Provenance

And every timeline is committed alongside it.
";

#[test]
fn segment_timings_equal_the_timeline_they_came_from() {
    let (out, m) = compiled_and_manifest(TWO_CHAPTERS);

    let from_timeline: Vec<(String, u64, u64)> = out
        .timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .map(|n| (n.line.clone(), n.start_ms, n.duration_ms))
        .collect();
    let from_manifest: Vec<(String, u64, u64)> = m
        .lines
        .iter()
        .map(|s| (s.id.clone(), s.start_ms, s.duration_ms))
        .collect();

    assert_eq!(
        from_timeline, from_manifest,
        "the manifest must not be able to disagree with the schedule it describes"
    );
    assert_eq!(m.duration_ms, out.timeline.duration_ms);
}

#[test]
fn chapters_carry_the_start_of_their_first_spoken_segment() {
    let m = manifest_for(TWO_CHAPTERS);
    assert_eq!(m.chapters.len(), 2);
    assert_eq!(m.chapters[0].id, "quick-start");
    assert_eq!(m.chapters[0].title, "Quick start");
    assert_eq!(m.chapters[0].start_ms, m.lines[0].start_ms);
    assert_eq!(m.chapters[1].id, "provenance");
    assert_eq!(m.chapters[1].start_ms, m.lines[1].start_ms);
}

/// Chapter slugs derive from titles, cannot be pinned, and are never
/// deduplicated, so two `# Setup` chapters share the slug `setup`. Joining
/// details to chapters by that slug made `min()` run over both chapters'
/// lines: the two markers came out with the same `start_ms` and the
/// second chapter's real start was lost. The join has to be positional.
#[test]
fn two_chapters_with_the_same_title_keep_their_own_starts() {
    // Explicit ids, because two identically-titled chapters would otherwise
    // derive the same line id and `assign_ids` would reject the script
    // before this join is ever reached. Pinning the ids is exactly what an
    // author hitting that error is told to do, so this is the shape the bug
    // actually reaches production in.
    let m = manifest_for(
        "\
# Setup

First chapter speaks here. {#setup-first}

# Setup

Second chapter speaks somewhere else entirely. {#setup-second}
",
    );

    assert_eq!(m.chapters.len(), 2, "two headings, two markers");
    assert_eq!(m.chapters[0].id, "setup");
    assert_eq!(m.chapters[1].id, "setup");
    assert_eq!(m.lines.len(), 2);

    assert_eq!(m.chapters[0].start_ms, m.lines[0].start_ms);
    assert_eq!(
        m.chapters[1].start_ms, m.lines[1].start_ms,
        "the second chapter's marker must be its own first line's start, \
         not the earlier chapter's"
    );
    assert_ne!(
        m.chapters[0].start_ms, m.chapters[1].start_ms,
        "two markers at the same instant would make the second chapter \
         unreachable from a navigation UI"
    );
}

#[test]
fn every_segment_names_the_chapter_it_was_spoken_in() {
    let m = manifest_for(TWO_CHAPTERS);
    assert_eq!(m.lines[0].chapter, "quick-start");
    assert_eq!(m.lines[1].chapter, "provenance");

    let json = serde_json::to_value(&m).unwrap();
    assert_eq!(
        json["lines"][0]["chapter"], "quick-start",
        "always serialized: reconstructing this from timestamps against a \
         `chapters` list that omits silent chapters is lossy"
    );
}

/// Pins current behaviour, deliberately. The scheduler subtracts a beat's
/// transition window from that beat before advancing its cursor, and for a
/// narration-only beat there is no action cue to absorb it, so the window
/// eats into speech and consecutive lines overlap. See spec §5.2, which
/// tells consumers to treat each line's own `duration_ms` as
/// authoritative for exactly this reason.
///
/// If the scheduler is ever changed so narration-only beats no longer
/// overlap, this test must fail loudly rather than the overlap silently
/// vanishing while §5.2 and the §7 consumer example still describe it.
#[test]
fn consecutive_narration_only_segments_abut_exactly() {
    let m = manifest_for(
        "\
# Quick start

One.

Two.

Three.
",
    );
    assert_eq!(m.lines.len(), 3);

    for pair in m.lines.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert_eq!(
            a.start_ms + a.duration_ms,
            b.start_ms,
            "`{}` and `{}` must neither overlap nor leave a gap: the auto \
             transition is capped at the quiet window (core spec §6.3), so a \
             script of plain paragraphs reads as continuous speech",
            a.id,
            b.id,
        );
    }
}

/// The contract still permits overlap (§5.2) — an author who sets a fixed
/// transition wider than the quiet window gets exactly what they asked for.
/// Pinned here because §5.2 tells consumers to expect it, and a claim in a
/// published contract with no test behind it decays.
#[test]
fn a_fixed_transition_can_still_make_segments_overlap() {
    let m = manifest_for(
        "\
---
output:
  transition: { kind: crossfade, duration: 2s }
---

# Quick start

One.

Two.
",
    );
    assert_eq!(m.lines.len(), 2);
    let (a, b) = (&m.lines[0], &m.lines[1]);
    assert!(
        a.start_ms + a.duration_ms > b.start_ms,
        "`{}` ends at {} and `{}` starts at {}",
        a.id,
        a.start_ms + a.duration_ms,
        b.id,
        b.start_ms,
    );
}

#[test]
fn a_chapter_with_no_narration_is_omitted() {
    let m = manifest_for(
        "\
# Intro

Only this chapter speaks.

# Silent
",
    );
    let ids: Vec<&str> = m.chapters.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["intro"],
        "a chapter marker with no start time would be meaningless to a consumer"
    );
}

#[test]
fn audio_paths_are_relative_to_the_manifest() {
    let m = manifest_for(TWO_CHAPTERS);
    assert_eq!(m.lines[0].audio, format!("audio/{}.wav", m.lines[0].id));
    assert!(
        !m.lines[0].audio.starts_with('/'),
        "an absolute path would not survive being served from anywhere else"
    );
}

#[test]
fn header_records_version_provenance_and_audio_shape() {
    let m = manifest_for(TWO_CHAPTERS);
    assert_eq!(m.manifest_version, MANIFEST_VERSION);
    assert_eq!(m.script, "tour.md");
    assert_eq!(m.locale, "en");
    assert_eq!(m.generated_by, "teleprompt 0.1.0");
    assert_eq!(m.audio.format, "wav");
    assert_eq!(m.audio.sample_rate, 48_000);
    assert_eq!(m.audio.channels, 1);
}

#[test]
fn downgrade_reason_is_present_as_null_but_words_are_omitted() {
    let m = manifest_for(TWO_CHAPTERS);
    let json = serde_json::to_value(&m).unwrap();
    let seg = &json["lines"][0];

    assert!(
        seg.get("downgrade_reason").is_some(),
        "consumers should not have to distinguish absent from null here"
    );
    assert!(seg["downgrade_reason"].is_null());
    assert!(
        seg.get("words").is_none(),
        "absent means the backend has no word timings; an empty array would be a lie"
    );
}

#[test]
fn words_from_the_backend_serialize_under_the_key_text_not_word() {
    let m = manifest_for_with_words(
        TWO_CHAPTERS,
        vec![
            WordTiming {
                word: "Every".to_string(),
                start_ms: 0,
                end_ms: 300,
            },
            WordTiming {
                word: "video".to_string(),
                start_ms: 300,
                end_ms: 600,
            },
        ],
    );
    let json = serde_json::to_value(&m).unwrap();

    assert_eq!(
        json["lines"][0]["words"],
        serde_json::json!([
            { "text": "Every", "start_ms": 0, "end_ms": 300 },
            { "text": "video", "start_ms": 300, "end_ms": 600 },
        ]),
        "WordTiming's field is `word`; WordEntry's is `text` — the join in \
         `build()` must perform that rename, not just carry the value through"
    );
}

#[test]
fn the_manifest_json_shape_is_stable() {
    let m = manifest_for(TWO_CHAPTERS);
    insta::assert_json_snapshot!(m);
}

/// The same contract with a scene in it. `TWO_CHAPTERS` is narration-only —
/// it exists to pin the chapter join — so on its own it would pin `beats` as
/// an empty array and let the published shape of a beat change unnoticed.
#[test]
fn the_manifest_json_shape_with_beats_is_stable() {
    insta::assert_json_snapshot!(manifest_for(ONE_BLOCK_SPLIT_BY_A_MARK));
}

#[test]
fn serialization_is_byte_stable() {
    let a = serde_json::to_string_pretty(&manifest_for(TWO_CHAPTERS)).unwrap();
    let b = serde_json::to_string_pretty(&manifest_for(TWO_CHAPTERS)).unwrap();
    assert_eq!(a, b);
}

/// The half of the manifest that lets a consumer place the *picture*.
///
/// Before `beats`, a consumer could place speech and nothing else, so
/// anything drawing a scene had to read the timeline — an explicitly
/// internal artifact — and join it to the manifest by document order.
mod beats {
    use super::*;

    const TWO_BEATS: &str = "# Chapter\n\n\
        First sentence. {#one}\n\n\
        ```teleprompt scene=mock\nwait 400ms\n```\n\n\
        Second sentence. {#two}\n\n\
        ```teleprompt scene=mock policy=concurrent\nwait 900ms\nmark\nwait 300ms\n```\n";

    #[test]
    fn every_scheduled_span_is_published() {
        let m = manifest_for(TWO_BEATS);
        let cues: Vec<&str> = m.beats.iter().map(|b| b.cue.as_str()).collect();
        assert_eq!(cues.len(), 3, "two blocks, one of them split by a mark");
        assert!(cues.iter().all(|s| s.contains('#')), "{cues:?}");
    }

    #[test]
    fn a_beat_carries_the_scheduled_numbers_not_the_adapters() {
        // The mock says `wait 900ms`; `concurrent` does not change that, so
        // the two agree here — the point is that the published number comes
        // from the timeline, which is what `fit-action` would move.
        let (out, m) = compiled_and_manifest(TWO_BEATS);
        for beat in &m.beats {
            let entry = out
                .timeline
                .entries
                .iter()
                .find(|e| e.action.as_ref().is_some_and(|a| a.cue == beat.cue))
                .expect("every beat comes from a timeline entry");
            let action = entry.action.as_ref().unwrap();
            assert_eq!(beat.start_ms, action.start_ms);
            assert_eq!(beat.duration_ms, action.duration_ms);
            assert_eq!(beat.policy, entry.policy);
            assert_eq!(beat.cue_hash, action.cue_hash);
        }
    }

    #[test]
    fn the_segment_spoken_over_a_span_is_named_and_unpaired_spans_say_null() {
        let m = manifest_for(TWO_BEATS);
        assert_eq!(m.beats[0].line.as_deref(), Some("one"));
        assert_eq!(m.beats[1].line.as_deref(), Some("two"));
        assert_eq!(
            m.beats[2].line, None,
            "a cue after a mark runs under what the policy left of the paragraph"
        );
    }

    #[test]
    fn a_pause_is_a_beat_rather_than_a_gap_to_infer() {
        let m = manifest_for(
            "# Chapter\n\nOnly sentence. {#one}\n\n\
             <!-- teleprompt: pause 600ms -->\n\n\
             ```teleprompt scene=mock\nwait 200ms\n```\n",
        );
        let pause = m
            .beats
            .iter()
            .find(|b| b.scene == "pause")
            .expect("a pause directive is published as a beat");
        assert_eq!(pause.duration_ms, 600);
        assert_eq!(pause.line, None);
    }

    #[test]
    fn beats_and_segments_agree_about_the_scene_and_adapter() {
        let m = manifest_for(TWO_BEATS);
        for beat in m.beats.iter().filter(|b| b.scene != "pause") {
            assert_eq!(beat.scene, "mock");
            assert_eq!(beat.adapter, "mock");
            assert_eq!(
                beat.duration_source, "exact",
                "the mock states its own timing"
            );
        }
    }

    #[test]
    fn the_version_says_beats_are_there() {
        // A v1 consumer that checks the version stops rather than silently
        // rendering speech over a blank screen.
        const _: () = assert!(MANIFEST_VERSION >= 2, "beats landed in v2");
        assert_eq!(manifest_for(TWO_BEATS).manifest_version, MANIFEST_VERSION);
    }
}
