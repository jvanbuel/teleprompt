use std::path::{Path, PathBuf};
use std::sync::Arc;

use teleprompt_cache::VoiceCache;
use teleprompt_compile::manifest::{self, AudioInfo, NarrationManifest, MANIFEST_VERSION};
use teleprompt_compile::manifest_diff::{self, ManifestDiff};
use teleprompt_compile::NarrationDetail;
use teleprompt_core::Hash;
use teleprompt_voice::{VoiceBackend, VoiceRegistry};
use teleprompt_voice_kokoro::KokoroVoice;

use crate::cmd::check::{cache_root, compile_script_with};
use crate::project::Project;

/// The `AudioInfo` sample rate for a locale that rendered no audio at all.
///
/// Every populated manifest takes its rate from the audio actually
/// produced; this is only what the field says when `segments` is empty and
/// there is nothing to describe. It is not a claim about any backend.
const NO_AUDIO_SAMPLE_RATE: u32 = 48_000;

/// A segment the fallback ladder could not deliver at the tier the script
/// asked for. Computed here rather than in `main.rs` so the policy question
/// — is a downgrade fatal? — is the only thing left for the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downgrade {
    pub segment_id: String,
    pub requested: String,
    pub actual: String,
    pub reason: String,
}

pub struct DubOutput {
    pub manifest: NarrationManifest,
    pub written: Vec<PathBuf>,
    pub warnings: Vec<String>,
    /// `Some` only under `--check`. `None` means nothing was compared.
    pub drift: Option<ManifestDiff>,
    /// Every segment whose delivered voice tier is not the requested one,
    /// in manifest order. Always populated; `--strict-voice` decides
    /// whether it is fatal.
    pub downgrades: Vec<Downgrade>,
}

/// Reads the downgrades straight off the published manifest, so what
/// `--strict-voice` fails on is exactly what a consumer would read.
fn downgrades_in(manifest: &NarrationManifest) -> Vec<Downgrade> {
    manifest
        .segments
        .iter()
        .filter(|s| s.voice_source != s.voice_source_actual)
        .map(|s| Downgrade {
            segment_id: s.id.clone(),
            requested: s.voice_source.clone(),
            actual: s.voice_source_actual.clone(),
            reason: s
                .downgrade_reason
                .clone()
                .unwrap_or_else(|| "no reason recorded".to_string()),
        })
        .collect()
}

pub fn render_downgrades(downgrades: &[Downgrade]) -> String {
    let mut s = String::new();
    for d in downgrades {
        s.push_str(&format!(
            "  {} — asked for {}, got {}: {}\n",
            d.segment_id, d.requested, d.actual, d.reason
        ));
    }
    s
}

/// Distinguishes a script that fails validation (exit 2) from a runtime
/// failure such as an unreadable committed manifest or a write error (exit
/// 1). A flat `Vec<String>` cannot carry that distinction, so `run_dub`
/// returns this instead of reusing `compile_script`'s error type directly.
pub enum DubError {
    Validation(Vec<String>),
    Runtime(String),
}

/// Where this locale's self-contained directory lives.
fn locale_dir(out_root: &Path, locale: &str) -> PathBuf {
    out_root.join(locale)
}

pub fn manifest_path(out_root: &Path, locale: &str) -> PathBuf {
    locale_dir(out_root, locale).join("narration.json")
}

/// Read a committed manifest, refusing a version this build does not
/// understand rather than letting serde fill in defaults and produce a
/// confident, wrong diff.
fn read_committed(path: &Path) -> Result<Option<NarrationManifest>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let probe: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let version = probe.get("manifest_version").and_then(|v| v.as_u64());
    if version != Some(MANIFEST_VERSION as u64) {
        return Err(format!(
            "{}: manifest_version {} is not supported (this build writes {})",
            path.display(),
            version
                .map(|v| v.to_string())
                .unwrap_or_else(|| "missing".into()),
            MANIFEST_VERSION,
        ));
    }
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

/// The duration the manifest will publish for `segment_id`.
///
/// Read off the timeline's narration entry, because that is precisely what
/// `manifest::build` copies into `SegmentEntry::duration_ms`. Deriving it
/// again from the synth result would compare a value against itself and
/// catch nothing.
///
/// `None` means the timeline has no narration entry for this segment, in
/// which case `build` drops it and there is no published duration to
/// disagree with.
fn published_duration_ms(
    timeline: &teleprompt_schedule::Timeline,
    segment_id: &str,
) -> Option<u64> {
    timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .find(|n| n.segment == segment_id)
        .map(|n| n.duration_ms)
}

/// `Some(message)` when a rendered segment is not the length the manifest
/// is about to publish for it.
///
/// This is the guard rail for the class of bug where the audio and the
/// number describing it come from two different places. It would have
/// caught `dub` rendering at the default speed while publishing a duration
/// measured at the script's configured speed, and it fires before anything
/// is written, so a mismatch never reaches an output directory.
///
/// The message names both values because "they differ" is not actionable —
/// which one is wrong is the whole question.
fn length_mismatch(segment_id: &str, actual_ms: u64, published_ms: u64) -> Option<String> {
    if actual_ms == published_ms {
        return None;
    }
    Some(format!(
        "segment `{segment_id}`: rendered audio is {actual_ms}ms but the manifest \
         publishes {published_ms}ms; a consumer placing this file at its stated \
         duration would clip or pad it"
    ))
}

/// One segment's audio, plus everything about how it was obtained that the
/// caller needs once every segment is in: the format it came out at (for
/// [`AudioInfo`]) and whether the cache entry it came from had to be healed
/// (for the author-facing warning). Kept as its own type — rather than
/// [`render_one`] returning the bare `(String, Vec<u8>, u64)` tuple `dub`
/// eventually wants — because under fan-out those two things can no longer
/// be folded into shared `Option`/`Vec` accumulators as segments finish:
/// several tasks finish at once, in no particular order, so each one has to
/// carry its own answer back rather than mutate a value the loop used to
/// own outright.
struct Rendered {
    segment_id: String,
    wav_bytes: Vec<u8>,
    rendered_ms: u64,
    sample_rate: u32,
    channels: u16,
    /// Set when the cache read that preceded this render was a corrupt
    /// sidecar rather than an ordinary miss — see `CacheRead::warning`.
    cache_warning: Option<String>,
}

/// Resolves one segment's audio: a cache hit already decided it, or a miss
/// decides it by calling `backend` and storing what comes back. Pulled out
/// of `run_dub_with`'s render step so each segment's work is a self-contained
/// unit a spawned task can own end to end.
async fn render_one(
    backend: &Arc<dyn VoiceBackend>,
    cache: &VoiceCache,
    detail: &NarrationDetail,
) -> Result<Rendered, DubError> {
    let read = cache
        .lookup(&detail.cache_key)
        .map_err(|e| DubError::Runtime(format!("segment `{}`: {e}", detail.segment_id)))?;
    let cache_warning = read.warning();

    let (wav_bytes, rendered_ms, sample_rate, channels) = match read.hit() {
        Some(cached) => (
            cached.wav,
            cached.duration_ms,
            cached.sample_rate,
            cached.channels,
        ),
        None => {
            // The request `compile` measured, not one rebuilt here.
            // Rebuilding it dropped `voice` and `speed`, so a script with
            // `voice: { speed: 2.0 }` published a duration from the
            // resolved config and a file rendered at the default. One
            // source of truth.
            let synthesized = backend
                .synthesize(&detail.synth_request)
                .await
                .map_err(|e| DubError::Runtime(format!("segment `{}`: {e}", detail.segment_id)))?;
            // Concurrent `store` calls are safe here: two tasks never share
            // a key (each segment appears once in `compiled.narration`), so
            // every pair of concurrent calls touches disjoint `<key>.wav`
            // and `<key>.json` paths. The one thing they do share is the
            // cache directory, and `create_dir_all` racing itself across
            // threads is fine — each call either creates it or observes it
            // already exists.
            let stored = cache
                .store(
                    &detail.cache_key,
                    &synthesized.pcm,
                    synthesized.word_timings.as_deref(),
                )
                .map_err(|e| DubError::Runtime(format!("segment `{}`: {e}", detail.segment_id)))?;
            (
                stored.wav,
                stored.duration_ms,
                stored.sample_rate,
                stored.channels,
            )
        }
    };

    Ok(Rendered {
        segment_id: detail.segment_id.clone(),
        wav_bytes,
        rendered_ms,
        sample_rate,
        channels,
        cache_warning,
    })
}

pub async fn run_dub(
    project: &Project,
    script: &Path,
    locale: &str,
    out_root: &Path,
    check_only: bool,
) -> Result<DubOutput, DubError> {
    // See the matching comment on `compile_script`: this is the project's
    // real `backends:` settings, not defaults — script front-matter-level
    // overrides do not reach construction here, for the same reason.
    let backends = project.config.backends.clone().unwrap_or_default();
    let registry =
        crate::voice::registry_for(&backends).map_err(|e| DubError::Validation(vec![e]))?;
    run_dub_with(&registry, project, script, locale, out_root, check_only).await
}

/// [`run_dub`] against a caller-supplied registry. See
/// [`crate::cmd::check::compile_script_with`] for why the seam exists.
pub async fn run_dub_with(
    registry: &VoiceRegistry,
    project: &Project,
    script: &Path,
    locale: &str,
    out_root: &Path,
    check_only: bool,
) -> Result<DubOutput, DubError> {
    // The backend the script's config resolved to, not one chosen here.
    // `detail.cache_key` was computed from this backend's `id()` and
    // `capabilities().version`, so synthesizing with any other one writes
    // the wrong audio into the content-addressed cache under this one's key
    // — permanently, and reported as `measured` by every later `plan`.
    let (compiled, backend) =
        compile_script_with(registry, project, script, locale).map_err(DubError::Validation)?;

    // Spec §7: the voice list is checked once here, not at `check` time.
    // `check` must stay offline and synchronous, and a gate that only works
    // when a server happens to be running is worse than no gate — the same
    // script would pass on one machine and fail on another.
    //
    // Downcasting rather than widening the trait: "list your voices" is not
    // something every backend can do, and adding an
    // `Option<Vec<String>>`-returning method to a single-method contract to
    // serve one implementation is exactly the speculative surface the
    // contract was sharpened to remove.
    if let Some(kokoro) = backend.as_any().downcast_ref::<KokoroVoice>() {
        let wanted = compiled
            .narration
            .iter()
            .filter_map(|d| d.synth_request.voice.clone())
            .collect::<std::collections::BTreeSet<_>>();
        if !wanted.is_empty() {
            // Spec §7.1: a server that is unreachable, slow, or returns
            // non-200 fails the command with exit 1 — that is a fact about
            // the machine, not the script. Only a server that *answered*
            // and simply does not list the configured voice is a script
            // problem (`Validation`, exit 2). `VoiceError`'s `Display`
            // already names the URL (`Client::fail` prefixes every message
            // with `kokoro at {base_url}: ...`), so both arms satisfy
            // §7.1's "names the URL" requirement without repeating it here.
            let available = kokoro
                .voices()
                .await
                .map_err(|e| DubError::Runtime(e.to_string()))?;
            let mut problems = Vec::new();
            for v in &wanted {
                if !available.contains(v) {
                    problems.push(format!(
                        "voice `{v}` is not available on the kokoro server at {} \
                         (available: {})",
                        kokoro.base_url(),
                        available.join(", ")
                    ));
                }
            }
            if !problems.is_empty() {
                return Err(DubError::Validation(problems));
            }
        }
    }

    // The same cache `compile_script` just read from, rooted the same way.
    // `compile` only ever looks a key up; `dub` is the one that fills a
    // miss in, by calling the backend and storing what comes back. `Arc`
    // rather than a borrow: the fan-out below hands a clone into every
    // spawned task, and `tokio::task::spawn` requires its future to be
    // `'static`, which a borrow of this local cannot be. `VoiceCache` is
    // just a `PathBuf` underneath, so the wrapping costs nothing.
    let cache = Arc::new(VoiceCache::new(cache_root(project)));

    // Bounded rather than unbounded: a local model server is the
    // bottleneck, and fanning out wider than it can serve makes the whole
    // run slower while making its failure modes worse. `null` has no
    // `concurrency` of its own — the downcast misses and `limit` falls back
    // to 1, which is a serial loop in every observable way.
    let limit = backend
        .as_any()
        .downcast_ref::<KokoroVoice>()
        .map(|k| k.concurrency())
        .unwrap_or(1);
    let permits = Arc::new(tokio::sync::Semaphore::new(limit));

    // Progress is reported in *completion* order with a running count, not
    // pretended into document order: under fan-out, which segment finishes
    // second is a fact about the server, not about the script, and a
    // counter that silently relabelled completions to look sequential would
    // be lying about what just happened. It is still per segment, because a
    // cold multi-segment script is otherwise minutes of silence (spec §15).
    let total = compiled.narration.len();
    let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut tasks: tokio::task::JoinSet<(usize, Result<Rendered, DubError>)> =
        tokio::task::JoinSet::new();
    for (i, detail) in compiled.narration.iter().enumerate() {
        let backend = backend.clone();
        let cache = cache.clone();
        let permits = permits.clone();
        let completed = completed.clone();
        let detail = detail.clone();
        tasks.spawn(async move {
            let _permit = permits
                .acquire_owned()
                .await
                .expect("semaphore is never closed");
            let r = render_one(&backend, &cache, &detail).await;
            if r.is_ok() {
                let n = completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                eprintln!("  [{n}/{total}] {} done", detail.segment_id);
            }
            (i, r)
        });
    }

    // `JoinSet` hands results back in completion order, not document order,
    // so each one is filed at the index its task carried rather than
    // appended. Spec §7.1: one segment's failure fails the run. The first
    // error to land aborts every task still in flight instead of waiting
    // for the rest to finish or fail too — a half-dubbed output directory
    // is worse than none, and there is no reason to keep hammering the
    // server once the run is already going to fail. Cache entries other
    // tasks already stored before the abort are left in place: the cache is
    // content-addressed and gitignored, so keeping what was already
    // synthesized is only useful, never wrong.
    let mut slots: Vec<Option<Rendered>> = (0..total).map(|_| None).collect();
    let mut failure: Option<DubError> = None;
    while let Some(joined) = tasks.join_next().await {
        let (i, r) = joined.expect("a render task panicked");
        match r {
            Ok(v) => slots[i] = Some(v),
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    if let Some(e) = failure {
        tasks.abort_all();
        return Err(e);
    }

    // Every slot was filled: the loop above only exits early (skipping some
    // indices) by taking the `failure` branch above, which already
    // returned.
    let rendered: Vec<Rendered> = slots
        .into_iter()
        .map(|r| r.expect("every index rendered when there was no failure"))
        .collect();

    // Each segment's rendered length rides along with its bytes: the length
    // guard cannot run here, because the number the manifest publishes is
    // not known until the recompile below.
    let mut audio: Vec<(String, Vec<u8>, u64)> = Vec::with_capacity(rendered.len());

    // Read off the audio actually produced rather than assumed from any one
    // backend. First *document-order* segment wins — `rendered` is already
    // sorted by index above, so this is a function of the script, not of
    // whichever request happened to answer first. The manifest has one
    // `AudioInfo` for the whole locale, so a backend that varied its rate
    // per segment would need a wider manifest, not a different pick here.
    let mut audio_format: Option<(u32, u16)> = None;

    // Cache entries `dub` had to re-render because they were unreadable.
    // Collected here rather than taken from either compile's warnings: the
    // recompile below runs against a cache this loop has already healed, so
    // by then there is nothing left to notice.
    let mut cache_warnings: Vec<String> = Vec::new();

    for r in rendered {
        if let Some(w) = &r.cache_warning {
            cache_warnings.push(format!("segment `{}`: {w}", r.segment_id));
        }
        audio_format.get_or_insert((r.sample_rate, r.channels));
        audio.push((r.segment_id, r.wav_bytes, r.rendered_ms));
    }

    // Every segment is warm now — the loop above either found it already
    // cached or just stored it. Recompile against that warm cache rather
    // than building the manifest from `compiled` above: that first compile
    // ran before anything was rendered, so on a cold project it published
    // `estimated` durations even though real audio was about to exist a few
    // lines later. Recompiling here is what makes `dub` idempotent — two
    // runs on an unedited script see an equally warm cache and produce
    // byte-identical manifests — and it costs one extra compile and no
    // synthesis, since by this point every lookup is a hit.
    let (compiled, _) =
        compile_script_with(registry, project, script, locale).map_err(DubError::Validation)?;

    // The guard rail, now that the number the manifest publishes exists.
    //
    // It used to run inside the loop above, against the *first* compile's
    // timeline — which on a cold cache holds estimates, not measurements.
    // With `null` that was invisible, because `NullVoice::synthesize` and
    // `WpmEstimator::estimate_ms` call the same function; against any
    // backend whose render differs from the word-count estimate it made the
    // first `dub` of every new segment fail, quoting a duration the manifest
    // would never have published, and the identical second run succeed off
    // the now-warm cache.
    //
    // Checked before anything is written to `--out`: an output directory
    // that disagrees with its own manifest is worse than no output at all.
    //
    // Be aware that as the code stands this cannot fire: the recompile above
    // reads back the very `duration_ms` that `cache.store` just wrote, so the
    // two numbers are the same value by construction. That is the point —
    // the invariant it asserts is currently upheld structurally, and the
    // guard is here to catch the day it stops being. The failure it would
    // catch is a key or metadata drift that makes the recompile resolve a
    // *different* entry than the one this run stored, which is silent and
    // unrecoverable at every layer above.
    for (segment_id, _, rendered_ms) in &audio {
        if let Some(published_ms) = published_duration_ms(&compiled.timeline, segment_id) {
            length_mismatch(segment_id, *rendered_ms, published_ms)
                .map_or(Ok(()), |m| Err(DubError::Runtime(m)))?;
        }
    }

    let (sample_rate, channels) = audio_format.unwrap_or((NO_AUDIO_SAMPLE_RATE, 1));
    let mut built = manifest::build(
        &compiled.timeline,
        &compiled.chapters,
        &compiled.narration,
        AudioInfo {
            format: "wav".to_string(),
            sample_rate,
            channels,
        },
    );

    // The two `audio_hash`es answer different questions, so the seeded value
    // has to be overwritten here rather than left alone.
    //
    // The timeline's is `Hash::of(cache_key)`: the *identity* of the audio a
    // segment resolves to. It moves when the segment would resolve to
    // different audio and stays put when `dub` re-renders the same audio,
    // which is exactly what a pacing drift check wants.
    //
    // The manifest's is `Hash::of(&wav_bytes)`: a description of the file
    // sitting beside it, which is what spec §5.1 documents and what lets a
    // consumer skip re-encoding a byte-identical render. `manifest::build`
    // runs before anything is encoded and can only seed the field with the
    // timeline's value, so `dub` — the one place that has the bytes —
    // publishes their hash.
    //
    // Done before the `--check` branch, not only on the write path, so a
    // comparison is always like-for-like.
    for (segment_id, bytes, _) in &audio {
        if let Some(seg) = built.segments.iter_mut().find(|s| s.id == *segment_id) {
            seg.audio_hash = Hash::of(bytes);
        }
    }

    let downgrades = downgrades_in(&built);

    // The re-render notices come first: they explain why anything below them
    // is being recomputed at all.
    let mut warnings = cache_warnings;
    warnings.extend(compiled.warnings.iter().cloned());

    if check_only {
        let committed =
            read_committed(&manifest_path(out_root, locale)).map_err(DubError::Runtime)?;
        let drift = match committed {
            Some(before) => manifest_diff::diff(&before, &built),
            // No manifest at all is maximal drift: everything is new.
            None => manifest_diff::diff(
                &NarrationManifest {
                    segments: Vec::new(),
                    chapters: Vec::new(),
                    duration_ms: 0,
                    ..built.clone()
                },
                &built,
            ),
        };
        return Ok(DubOutput {
            manifest: built,
            written: Vec::new(),
            warnings: warnings.clone(),
            drift: Some(drift),
            downgrades,
        });
    }

    let dir = locale_dir(out_root, locale);
    let audio_dir = dir.join("audio");
    std::fs::create_dir_all(&audio_dir)
        .map_err(|e| DubError::Runtime(format!("cannot create {}: {e}", audio_dir.display())))?;

    let mut written = Vec::new();
    for (segment_id, bytes, _) in &audio {
        let path = dir.join(manifest::audio_path(segment_id, "wav"));
        std::fs::write(&path, bytes)
            .map_err(|e| DubError::Runtime(format!("cannot write {}: {e}", path.display())))?;
        written.push(path);
    }

    let path = manifest_path(out_root, locale);
    let json = serde_json::to_string_pretty(&built)
        .map_err(|e| DubError::Runtime(format!("cannot serialize manifest: {e}")))?;
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| DubError::Runtime(format!("cannot write {}: {e}", path.display())))?;
    written.push(path);

    Ok(DubOutput {
        manifest: built,
        written,
        warnings,
        drift: None,
        downgrades,
    })
}

pub fn render_dub(out: &DubOutput) -> String {
    let mut s = format!(
        "{} ({}) — {} segment(s), {:.1}s\n",
        out.manifest.script,
        out.manifest.locale,
        out.manifest.segments.len(),
        out.manifest.duration_ms as f64 / 1000.0,
    );
    for path in &out.written {
        s.push_str(&format!("  wrote {}\n", path.display()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use teleprompt_compile::manifest::SegmentEntry;
    use teleprompt_core::Hash;

    #[test]
    fn matching_lengths_pass_the_guard() {
        assert_eq!(length_mismatch("welcome", 3250, 3250), None);
    }

    /// The exact shape of C1: a manifest publishing 3250ms beside a file of
    /// 6500ms. The guard has to name the segment and both numbers, because
    /// which of the two is wrong is the whole diagnosis.
    #[test]
    fn a_mismatch_names_the_segment_and_both_lengths() {
        let msg = length_mismatch("welcome", 6500, 3250).expect("must be caught");
        assert!(msg.contains("welcome"), "{msg}");
        assert!(msg.contains("6500ms"), "{msg}");
        assert!(msg.contains("3250ms"), "{msg}");
    }

    fn segment(id: &str, requested: &str, actual: &str, reason: Option<&str>) -> SegmentEntry {
        SegmentEntry {
            id: id.to_string(),
            text: String::new(),
            chapter: "a".to_string(),
            start_ms: 0,
            duration_ms: 0,
            duration_source: "measured".to_string(),
            audio: String::new(),
            voice_source: requested.to_string(),
            voice_source_actual: actual.to_string(),
            downgrade_reason: reason.map(str::to_string),
            source_hash: Hash::of(b""),
            audio_hash: Hash::of(b""),
            words: None,
        }
    }

    fn manifest_with(segments: Vec<SegmentEntry>) -> NarrationManifest {
        NarrationManifest {
            manifest_version: MANIFEST_VERSION,
            script: "s.md".to_string(),
            locale: "en".to_string(),
            generated_by: "teleprompt test".to_string(),
            duration_ms: 0,
            audio: AudioInfo {
                format: "wav".to_string(),
                sample_rate: 48_000,
                channels: 1,
            },
            chapters: Vec::new(),
            segments,
        }
    }

    #[test]
    fn a_segment_delivered_at_the_requested_tier_is_not_a_downgrade() {
        let m = manifest_with(vec![segment("a", "synthetic", "synthetic", None)]);
        assert!(downgrades_in(&m).is_empty());
    }

    #[test]
    fn a_downgrade_carries_both_tiers_and_the_reason() {
        let m = manifest_with(vec![
            segment("a", "synthetic", "synthetic", None),
            segment("b", "recorded", "synthetic", Some("no takes recorded")),
        ]);
        assert_eq!(
            downgrades_in(&m),
            vec![Downgrade {
                segment_id: "b".to_string(),
                requested: "recorded".to_string(),
                actual: "synthetic".to_string(),
                reason: "no takes recorded".to_string(),
            }]
        );
        let rendered = render_downgrades(&downgrades_in(&m));
        assert!(rendered.contains("b"), "{rendered}");
        assert!(rendered.contains("no takes recorded"), "{rendered}");
    }
}
