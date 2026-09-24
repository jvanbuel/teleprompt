use std::path::{Path, PathBuf};
use std::sync::Arc;

use teleprompt_cache::VoiceCache;
use teleprompt_compile::manifest::{self, AudioInfo, NarrationManifest, MANIFEST_VERSION};
use teleprompt_compile::manifest_diff::{self, ManifestDiff};
use teleprompt_compile::NarrationDetail;
use teleprompt_core::config::OutputConfig;
use teleprompt_core::Hash;
use teleprompt_voice::VoiceBackend;

use crate::cmd::check::{cache_root, compile_script_with};
use crate::project::Project;
use crate::voice::Backends;

/// The `AudioInfo` sample rate for a locale that rendered no audio at all.
///
/// Every populated manifest takes its rate from the audio actually
/// produced; this is only what the field says when `lines` is empty and
/// there is nothing to describe. It is not a claim about any backend.
const NO_AUDIO_SAMPLE_RATE: u32 = 48_000;

/// A line the fallback ladder could not deliver at the tier the script
/// asked for. Computed here rather than in `main.rs` so the policy question
/// — is a downgrade fatal? — is the only thing left for the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downgrade {
    pub line_id: String,
    pub requested: String,
    pub actual: String,
    pub reason: String,
}

pub struct DubOutput {
    pub manifest: NarrationManifest,
    /// Every shot's published source, which is what a capture backend
    /// runs. The manifest names the items; only this says what they do.
    pub shots: Vec<teleprompt_compile::ShotSource>,
    /// The scenes as configured. A capture backend has to know what
    /// terminal it is opening.
    pub scenes: std::collections::BTreeMap<String, teleprompt_core::config::SceneConfig>,
    /// The frame the script asked for. Carried through because `build`
    /// renders what `dub` published, and the script's `output:` block is
    /// not part of the narration manifest.
    pub output: OutputConfig,
    pub written: Vec<PathBuf>,
    pub warnings: Vec<String>,
    /// `Some` only under `--check`. `None` means nothing was compared.
    pub drift: Option<ManifestDiff>,
    /// Every line whose delivered voice tier is not the requested one,
    /// in manifest order. Always populated; `--strict-voice` decides
    /// whether it is fatal.
    pub downgrades: Vec<Downgrade>,
}

/// Reads the downgrades straight off the published manifest, so what
/// `--strict-voice` fails on is exactly what a consumer would read.
fn downgrades_in(manifest: &NarrationManifest) -> Vec<Downgrade> {
    manifest
        .lines
        .iter()
        .filter(|s| s.voice_source != s.voice_source_actual)
        .map(|s| Downgrade {
            line_id: s.id.clone(),
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
            d.line_id, d.requested, d.actual, d.reason
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

/// The duration the manifest will publish for `line_id`.
///
/// Read off the timeline's narration entry, because that is precisely what
/// `manifest::build` copies into `LineEntry::duration_ms`. Deriving it
/// again from the synth result would compare a value against itself and
/// catch nothing.
///
/// `None` means the timeline has no narration entry for this line, in
/// which case `build` drops it and there is no published duration to
/// disagree with.
fn published_duration_ms(timeline: &teleprompt_schedule::Timeline, line_id: &str) -> Option<u64> {
    timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .find(|n| n.line == line_id)
        .map(|n| n.duration_ms)
}

/// `Some(message)` when a rendered line is not the length the manifest
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
fn length_mismatch(line_id: &str, actual_ms: u64, published_ms: u64) -> Option<String> {
    if actual_ms == published_ms {
        return None;
    }
    Some(format!(
        "line `{line_id}`: rendered audio is {actual_ms}ms but the manifest \
         publishes {published_ms}ms; a consumer placing this file at its stated \
         duration would clip or pad it"
    ))
}

/// What rendering one *cache key* produced: the audio, the format it came
/// out at (for [`AudioInfo`]), and whether the cache entry it came from had
/// to be healed (for the author-facing warning).
///
/// Deliberately keyed to a `CacheKey`, not a line — `teleprompt_cache::key`
/// hashes backend id/version, locale, voice, speed, and the narration
/// *text*, not the line id, so two lines with identical text resolve
/// to the same key and must resolve to the same [`RenderedAudio`]. Rendering
/// each occurrence independently would double-count synthesis work against
/// the exact bottleneck fan-out exists to relieve, and would put two tasks
/// of this run in a `store` race that the cache would have to arbitrate
/// rather than one this run never enters. See `run_dub_with`'s grouping
/// step, which is what guarantees there is only ever one task per key and
/// therefore only ever one `render_one` call per key.
struct RenderedAudio {
    wav_bytes: Vec<u8>,
    rendered_ms: u64,
    sample_rate: u32,
    channels: u16,
    /// Set when the cache read that preceded this render was a corrupt
    /// sidecar rather than an ordinary miss — see `CacheRead::warning`.
    cache_warning: Option<String>,
}

/// One line's audio: [`RenderedAudio`] plus the identity of the line
/// it is published under. Several lines can point at the same
/// `RenderedAudio` (identical narration text), each getting its own
/// `Rendered` with its own `line_id` — the manifest still publishes one
/// entry per line even though only one render happened.
struct Rendered {
    line_id: String,
    wav_bytes: Vec<u8>,
    rendered_ms: u64,
    sample_rate: u32,
    channels: u16,
    cache_warning: Option<String>,
}

/// Resolves one *cache key*'s audio: a cache hit already decided it, or a
/// miss decides it by calling `backend` and storing what comes back.
/// `detail` is any one narration detail that resolves to this key — when
/// several lines share a key, the caller picks one representative before
/// calling this, since every one of them would produce an identical
/// `SynthRequest` and therefore an identical result. Pulled out of
/// `run_dub_with`'s render step so each key's work is a self-contained unit
/// a spawned task can own end to end.
async fn render_one(
    backend: &Arc<dyn VoiceBackend>,
    cache: &VoiceCache,
    detail: &NarrationDetail,
) -> Result<RenderedAudio, DubError> {
    let read = cache
        .lookup(&detail.cache_key)
        .map_err(|e| DubError::Runtime(format!("line `{}`: {e}", detail.line_id)))?;
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
                .map_err(|e| DubError::Runtime(format!("line `{}`: {e}", detail.line_id)))?;
            // Exactly one task of *this* run ever calls `store` for a given
            // key — see the grouping step in `run_dub_with`. Another
            // `teleprompt dub` process on the same project is a different
            // matter, and `VoiceCache::store` is what arbitrates that: it
            // publishes atomically, and a writer that loses the race returns
            // the entry that won, so the bytes below and the sidecar the
            // recompile reads are always the same entry.
            let stored = cache
                .store(
                    &detail.cache_key,
                    &synthesized.pcm,
                    synthesized.word_timings.as_deref(),
                )
                .map_err(|e| DubError::Runtime(format!("line `{}`: {e}", detail.line_id)))?;
            (
                stored.wav,
                stored.duration_ms,
                stored.sample_rate,
                stored.channels,
            )
        }
    };

    Ok(RenderedAudio {
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
    let backends = crate::cmd::check::backends_of(project);
    run_dub_with(&backends, project, script, locale, out_root, check_only).await
}

/// [`run_dub`] against caller-supplied backends. See
/// [`crate::cmd::check::compile_script_with`] for why the seam exists.
pub async fn run_dub_with(
    backends: &Backends,
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
        compile_script_with(backends, project, script, locale).map_err(DubError::Validation)?;

    let cache = Arc::new(VoiceCache::new(cache_root(project)));

    // Whether this run has anything to ask a server for. A project whose
    // every line is already cached needs no server at all, and saying so
    // is what makes "build it once, then iterate on the cache" a workflow
    // rather than something you can only do while a GPU box answers. A
    // corrupt entry counts as missing: it is about to be re-synthesized.
    let anything_to_synthesize = compiled.narration.iter().any(|detail| {
        !matches!(
            cache.lookup_meta(&detail.cache_key),
            Ok(teleprompt_cache::CacheRead::Hit(_))
        )
    });

    // The voice list is checked once here, not at `check` time. `check` must
    // stay offline and synchronous, and a gate that only works when a server
    // happens to be running is worse than no gate — the same script would pass
    // on one machine and fail on another.
    //
    // `Backends::kokoro` rather than widening the trait: "list your voices"
    // is not something every backend can do, and adding an
    // `Option<Vec<String>>`-returning method to a single-method contract to
    // serve one implementation is exactly the speculative surface the
    // contract was sharpened to remove. `backends` already built the
    // concrete `KokoroVoice` this project's settings describe, so asking it
    // for that handle by the resolved backend's id is the same information
    // a downcast on `backend` would recover, without needing `backend` to
    // carry its own concrete type at runtime.
    if let Some(kokoro) = backends
        .kokoro(backend.id())
        .filter(|_| anything_to_synthesize)
    {
        let wanted = compiled
            .narration
            .iter()
            .filter_map(|d| d.synth_request.voice.clone())
            .collect::<std::collections::BTreeSet<_>>();
        if !wanted.is_empty() {
            // A server that is unreachable, slow, or returns non-200 fails the
            // command with exit 1 — that is a fact about the machine, not the
            // script. Only a server that *answered* and simply does not list
            // the configured voice is a script problem (`Validation`, exit 2).
            // `VoiceError`'s `Display` already names the URL (`Client::fail`
            // prefixes every message with `kokoro at {base_url}: ...`), so both
            // arms satisfy the "names the URL" rule
            // (docs/design.md#backend-failure).
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
    // Bounded rather than unbounded: a local model server is the
    // bottleneck, and fanning out wider than it can serve makes the whole
    // run slower while making its failure modes worse. `concurrency` is a
    // pure configuration value — `backends` read it out of this project's
    // settings when it built `KokoroVoice`, so it is read the same way here
    // rather than asked of the backend after the fact. `null` (and any
    // backend with no entry in `backends`) has no `concurrency` of its own —
    // `Backends::kokoro` returns `None` and `limit` falls back to 1. At
    // `limit = 1` the semaphore admits exactly one task at a time, and —
    // because spawn order below is document order (see the sort on
    // `groups`) — that one task is always the earliest-document-order group
    // still waiting. On the CLI's current-thread runtime that chain (spawn
    // order → poll order → acquire order → completion order) has no room
    // for anything to reorder it, which is what makes `limit = 1` a serial
    // loop in every observable way, including stderr: verified by running a
    // six-line `null` project's `dub` six times in a row and diffing the
    // printed line-id sequence, not merely asserted.
    let limit = backends
        .kokoro(backend.id())
        .map(|k| k.concurrency())
        .unwrap_or(1);
    let permits = Arc::new(tokio::sync::Semaphore::new(limit));

    // Progress is reported in *completion* order with a running count, not
    // pretended into document order: under genuine concurrency (`limit > 1`),
    // which line finishes second is a fact about the server, not about the
    // script, and a counter that silently relabelled completions to look
    // sequential would be lying about what just happened. At `limit = 1` there
    // is no concurrency for completion order to diverge from document order in
    // the first place — see the note on `limit` above — so this order is
    // document order there too, not a special case, just the one case where
    // "completion order" and "document order" happen to coincide. Progress is
    // still per line, because a cold multi-line script is otherwise minutes of
    // silence.
    let total = compiled.narration.len();
    let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Grouped by cache key, not spawned one task per line: two lines
    // with identical narration text resolve to the same `CacheKey` (see
    // `RenderedAudio`'s doc comment), so one task renders each *distinct*
    // key and its result is fanned out to every line index that shares
    // it. This is what makes rendering identical text once rather than
    // once per occurrence, and it is what removes the same-key concurrent
    // `store` race by construction — there is exactly one task per key, so
    // there is never a second writer to race.
    let mut groups: std::collections::HashMap<teleprompt_cache::CacheKey, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, detail) in compiled.narration.iter().enumerate() {
        groups.entry(detail.cache_key.clone()).or_default().push(i);
    }

    // `HashMap` iteration order is an arbitrary, per-process-random
    // permutation (`RandomState`), so spawning in `groups.into_values()`
    // order — as an earlier version of this function did — made stderr
    // output a different, non-reproducible ordering on every run, on every
    // backend including `null`. Each group's indices are already ascending
    // (pushed in the `for (i, ...)` loop above), so sorting the groups
    // themselves by their first index restores document order as the spawn
    // order, for free, with no dependency on `HashMap`'s iteration order.
    let mut groups: Vec<Vec<usize>> = groups.into_values().collect();
    groups.sort_by_key(|indices| indices[0]);

    let mut tasks: tokio::task::JoinSet<(Vec<usize>, Result<RenderedAudio, DubError>)> =
        tokio::task::JoinSet::new();
    for indices in groups {
        // Any member of the group is a valid representative: identical
        // cache keys imply identical `SynthRequest`s (same backend/version,
        // same locale/voice/speed, same text — that is exactly what the key
        // is a hash of), so whichever one gets rendered is correct for
        // every index in the group.
        let detail = compiled.narration[indices[0]].clone();
        let line_ids: Vec<String> = indices
            .iter()
            .map(|&i| compiled.narration[i].line_id.clone())
            .collect();
        let backend = backend.clone();
        let cache = cache.clone();
        let permits = permits.clone();
        let completed = completed.clone();
        tasks.spawn(async move {
            let _permit = permits
                .acquire_owned()
                .await
                .expect("semaphore is never closed");
            let r = render_one(&backend, &cache, &detail).await;
            if r.is_ok() {
                // One "done" line per line the group covers, not one per
                // request: progress is a promise to the author about their
                // script, and a script with two identical sentences still
                // has two lines to account for, even though only one of
                // them made a network call.
                for id in &line_ids {
                    let n = completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    eprintln!("  [{n}/{total}] {id} done");
                }
            }
            (indices, r)
        });
    }

    // `JoinSet` hands results back in completion order, not document order, so
    // each one is filed at the indices its task carried rather than appended.
    // One line's failure fails the run. The first error to land aborts every
    // task still in flight instead of waiting for the rest to finish or fail
    // too — a half-dubbed output directory is worse than none, and there is no
    // reason to keep hammering the server once the run is already going to
    // fail. Cache entries other tasks already stored before the abort are left
    // in place: the cache is content-addressed and gitignored, so keeping what
    // was already synthesized is only useful, never wrong.
    let mut slots: Vec<Option<Rendered>> = (0..total).map(|_| None).collect();
    let mut failure: Option<DubError> = None;
    while let Some(joined) = tasks.join_next().await {
        let (indices, r) = joined.expect("a render task panicked");
        match r {
            Ok(audio) => {
                for i in indices {
                    slots[i] = Some(Rendered {
                        line_id: compiled.narration[i].line_id.clone(),
                        wav_bytes: audio.wav_bytes.clone(),
                        rendered_ms: audio.rendered_ms,
                        sample_rate: audio.sample_rate,
                        channels: audio.channels,
                        cache_warning: audio.cache_warning.clone(),
                    });
                }
            }
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

    // Each line's rendered length rides along with its bytes: the length
    // guard cannot run here, because the number the manifest publishes is
    // not known until the recompile below.
    let mut audio: Vec<(String, Vec<u8>, u64)> = Vec::with_capacity(rendered.len());

    // Read off the audio actually produced rather than assumed from any one
    // backend. First *document-order* line wins — `rendered` is already
    // sorted by index above, so this is a function of the script, not of
    // whichever request happened to answer first. The manifest has one
    // `AudioInfo` for the whole locale, so a backend that varied its rate
    // per line would need a wider manifest, not a different pick here.
    let mut audio_format: Option<(u32, u16)> = None;

    // Cache entries `dub` had to re-render because they were unreadable.
    // Collected here rather than taken from either compile's warnings: the
    // recompile below runs against a cache this loop has already healed, so
    // by then there is nothing left to notice.
    let mut cache_warnings: Vec<String> = Vec::new();

    for r in rendered {
        if let Some(w) = &r.cache_warning {
            cache_warnings.push(format!("line `{}`: {w}", r.line_id));
        }
        audio_format.get_or_insert((r.sample_rate, r.channels));
        audio.push((r.line_id, r.wav_bytes, r.rendered_ms));
    }

    // Every line is warm now — the loop above either found it already
    // cached or just stored it. Recompile against that warm cache rather
    // than building the manifest from `compiled` above: that first compile
    // ran before anything was rendered, so on a cold project it published
    // `estimated` durations even though real audio was about to exist a few
    // lines later. Recompiling here is what makes `dub` idempotent — two
    // runs on an unedited script see an equally warm cache and produce
    // byte-identical manifests — and it costs one extra compile and no
    // synthesis, since by this point every lookup is a hit.
    let (compiled, _) =
        compile_script_with(backends, project, script, locale).map_err(DubError::Validation)?;

    // The guard rail, now that the number the manifest publishes exists.
    //
    // It used to run inside the loop above, against the *first* compile's
    // timeline — which on a cold cache holds estimates, not measurements.
    // With `null` that was invisible, because `NullVoice::synthesize` and
    // `WpmEstimator::estimate_ms` call the same function; against any
    // backend whose render differs from the word-count estimate it made the
    // first `dub` of every new line fail, quoting a duration the manifest
    // would never have published, and the identical second run succeed off
    // the now-warm cache.
    //
    // Checked before anything is written to `--out`: an output directory
    // that disagrees with its own manifest is worse than no output at all.
    //
    // An earlier version of this comment claimed the guard could not fire,
    // on the grounds that the recompile reads back the very `duration_ms`
    // that `cache.store` just wrote. That was only ever true of a single
    // `dub`: a second process storing the same key could replace the sidecar
    // between the store and the recompile, and the guard then fired on a run
    // that had done nothing wrong. `VoiceCache::store` now returns whichever
    // entry was published rather than always the caller's own, so the two
    // numbers are again the same entry's — but "cannot fire" is a claim
    // about a whole system and this one is a guard rail, so it is written to
    // catch the day that stops holding. The failure it exists for is a key
    // or metadata drift that makes the recompile resolve a *different* entry
    // than the one this run stored, which is silent and unrecoverable at
    // every layer above.
    for (line_id, _, rendered_ms) in &audio {
        if let Some(published_ms) = published_duration_ms(&compiled.timeline, line_id) {
            length_mismatch(line_id, *rendered_ms, published_ms)
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
    // line resolves to. It moves when the line would resolve to
    // different audio and stays put when `dub` re-renders the same audio,
    // which is exactly what a pacing drift check wants.
    //
    // The manifest's is `Hash::of(&wav_bytes)`: a description of the file
    // sitting beside it (docs/design.md#manifest), which is what lets a
    // consumer skip re-encoding a byte-identical render. `manifest::build` runs
    // before anything is encoded and can only seed the field with the
    // timeline's value, so `dub` — the one place that has the bytes — publishes
    // their hash.
    //
    // Done before the `--check` branch, not only on the write path, so a
    // comparison is always like-for-like.
    for (line_id, bytes, _) in &audio {
        if let Some(seg) = built.lines.iter_mut().find(|s| s.id == *line_id) {
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
                    lines: Vec::new(),
                    chapters: Vec::new(),
                    duration_ms: 0,
                    ..built.clone()
                },
                &built,
            ),
        };
        return Ok(DubOutput {
            manifest: built,
            shots: compiled.shots.clone(),
            scenes: compiled.scenes.clone(),
            output: compiled.output.clone(),
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
    for (line_id, bytes, _) in &audio {
        let path = dir.join(manifest::audio_path(line_id, "wav"));
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
        shots: compiled.shots.clone(),
        scenes: compiled.scenes.clone(),
        output: compiled.output.clone(),
        written,
        warnings,
        drift: None,
        downgrades,
    })
}

pub fn render_dub(out: &DubOutput) -> String {
    let mut s = format!(
        "{} ({}) — {} line(s), {:.1}s\n",
        out.manifest.script,
        out.manifest.locale,
        out.manifest.lines.len(),
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
    use teleprompt_compile::manifest::LineEntry;
    use teleprompt_core::Hash;

    #[test]
    fn matching_lengths_pass_the_guard() {
        assert_eq!(length_mismatch("welcome", 3250, 3250), None);
    }

    /// The exact shape of C1: a manifest publishing 3250ms beside a file of
    /// 6500ms. The guard has to name the line and both numbers, because
    /// which of the two is wrong is the whole diagnosis.
    #[test]
    fn a_mismatch_names_the_line_and_both_lengths() {
        let msg = length_mismatch("welcome", 6500, 3250).expect("must be caught");
        assert!(msg.contains("welcome"), "{msg}");
        assert!(msg.contains("6500ms"), "{msg}");
        assert!(msg.contains("3250ms"), "{msg}");
    }

    fn line(id: &str, requested: &str, actual: &str, reason: Option<&str>) -> LineEntry {
        LineEntry {
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

    fn manifest_with(lines: Vec<LineEntry>) -> NarrationManifest {
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
            lines,
            shots: Vec::new(),
        }
    }

    #[test]
    fn a_line_delivered_at_the_requested_tier_is_not_a_downgrade() {
        let m = manifest_with(vec![line("a", "synthetic", "synthetic", None)]);
        assert!(downgrades_in(&m).is_empty());
    }

    #[test]
    fn a_downgrade_carries_both_tiers_and_the_reason() {
        let m = manifest_with(vec![
            line("a", "synthetic", "synthetic", None),
            line("b", "recorded", "synthetic", Some("no takes recorded")),
        ]);
        assert_eq!(
            downgrades_in(&m),
            vec![Downgrade {
                line_id: "b".to_string(),
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
