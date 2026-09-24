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

/// The `AudioInfo` sample rate for a locale with no lines. Any other
/// manifest takes its rate from the audio produced.
const NO_AUDIO_SAMPLE_RATE: u32 = 48_000;

/// A line the fallback ladder could not deliver at the requested tier
/// (docs/design.md#voice-tiers). Whether that is fatal is the caller's call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downgrade {
    pub line_id: String,
    pub requested: String,
    pub actual: String,
    pub reason: String,
}

pub struct DubOutput {
    pub manifest: NarrationManifest,
    /// Every shot's source, which a capture backend runs. The manifest
    /// names the shots; only this says what they do.
    pub shots: Vec<teleprompt_compile::ShotSource>,
    /// The scenes as configured, which a capture backend opens.
    pub scenes: std::collections::BTreeMap<String, teleprompt_core::config::SceneConfig>,
    /// The script's `output:` block, which `build` needs and the manifest
    /// does not carry.
    pub output: OutputConfig,
    pub written: Vec<PathBuf>,
    pub warnings: Vec<String>,
    /// `Some` only under `--check`. `None` means nothing was compared.
    pub drift: Option<ManifestDiff>,
    /// Every line delivered below its requested tier, in manifest order.
    /// Always populated; `--strict-voice` decides whether it is fatal.
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

/// A script that fails validation (exit 2) or a runtime failure such as an
/// unreadable committed manifest or a write error (exit 1).
pub enum DubError {
    Validation(Vec<String>),
    Runtime(String),
}

fn locale_dir(out_root: &Path, locale: &str) -> PathBuf {
    out_root.join(locale)
}

pub fn manifest_path(out_root: &Path, locale: &str) -> PathBuf {
    locale_dir(out_root, locale).join("narration.json")
}

/// Refuses a `manifest_version` this build does not write, rather than
/// letting serde fill in defaults and produce a confident, wrong diff.
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

/// The duration the manifest will publish for `line_id`, read off the
/// timeline because that is what `manifest::build` copies. Deriving it from
/// the synth result would compare a value against itself. `None`: the line
/// has no narration entry, so `build` drops it and publishes nothing.
fn published_duration_ms(timeline: &teleprompt_schedule::Timeline, line_id: &str) -> Option<u64> {
    timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .find(|n| n.line == line_id)
        .map(|n| n.duration_ms)
}

/// `Some(message)` when a rendered line is not the length the manifest is
/// about to publish for it: the audio and the number describing it came
/// from two places. The message names both values, because which one is
/// wrong is the whole question.
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

/// What rendering one *cache key* produced, and whether its cache entry
/// had to be healed.
///
/// Keyed to a key, not a line: the key hashes the text, not the line id
/// (docs/design.md#voice-cache), so two lines with identical text share
/// one render. `run_dub_with` groups lines by key so each key is rendered
/// once, by one task, and this run never races itself in `store`.
struct RenderedAudio {
    wav_bytes: Vec<u8>,
    rendered_ms: u64,
    sample_rate: u32,
    channels: u16,
    /// See `CacheRead::warning`.
    cache_warning: Option<String>,
}

/// [`RenderedAudio`] under the line it is published as. Lines sharing a
/// key each get their own, since the manifest has one entry per line.
struct Rendered {
    line_id: String,
    wav_bytes: Vec<u8>,
    rendered_ms: u64,
    sample_rate: u32,
    channels: u16,
    cache_warning: Option<String>,
}

/// One cache key's audio, from the cache or, on a miss, from `backend`
/// and then stored. `detail` is any line that resolves to the key: equal
/// keys imply identical `SynthRequest`s.
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
            // The request `compile` measured, not one rebuilt here, so the
            // audio and the published duration share one source of truth.
            let synthesized = backend
                .synthesize(&detail.synth_request)
                .await
                .map_err(|e| DubError::Runtime(format!("line `{}`: {e}", detail.line_id)))?;
            // Another `dub` process may store this key concurrently;
            // `VoiceCache::store` returns whichever entry won, so these bytes
            // and the sidecar the recompile reads are the same entry.
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
    // The project's `backends:` settings; a script's front-matter override
    // does not reach construction, as in `compile_script`.
    let backends = crate::cmd::check::backends_of(project);
    run_dub_with(&backends, project, script, locale, out_root, check_only).await
}

/// [`run_dub`] against caller-supplied backends; the seam exists for the
/// reason `compile_script_with` gives.
pub async fn run_dub_with(
    backends: &Backends,
    project: &Project,
    script: &Path,
    locale: &str,
    out_root: &Path,
    check_only: bool,
) -> Result<DubOutput, DubError> {
    // Synthesize only with the backend the keys were computed from; see
    // `compile_script_with`.
    let (compiled, backend) =
        compile_script_with(backends, project, script, locale).map_err(DubError::Validation)?;
    let cache = Arc::new(VoiceCache::new(cache_root(project)));

    check_voices(backends, backend.id(), &compiled, &cache).await?;
    let limit = backends
        .kokoro(backend.id())
        .map(|k| k.concurrency())
        .unwrap_or(1);
    let rendered = render_all(&backend, &cache, &compiled.narration, limit).await?;
    let audio = Audio::collect(rendered);

    // Recompile against the now-warm cache: the first compile ran before
    // anything was rendered, so on a cold project it holds estimates. This
    // makes `dub` idempotent, and costs no synthesis.
    let (compiled, _) =
        compile_script_with(backends, project, script, locale).map_err(DubError::Validation)?;
    audio.check_lengths(&compiled.timeline)?;
    let built = audio.manifest(&compiled);
    let downgrades = downgrades_in(&built);

    // The re-render notices come first: they explain why anything below them
    // is being recomputed at all.
    let mut warnings = audio.cache_warnings.clone();
    warnings.extend(compiled.warnings.iter().cloned());

    let (written, drift) = if check_only {
        (
            Vec::new(),
            Some(drift_from_committed(out_root, locale, &built)?),
        )
    } else {
        (write_output(out_root, locale, &audio, &built)?, None)
    };
    Ok(DubOutput {
        manifest: built,
        shots: compiled.shots.clone(),
        scenes: compiled.scenes.clone(),
        output: compiled.output.clone(),
        written,
        warnings,
        drift,
        downgrades,
    })
}

/// Checks the voices the script asks for against the server, once, before
/// anything is synthesized.
///
/// Here, not in `check`: `check` stays offline, and a gate that depends on a
/// running server would pass a script on one machine and fail it on
/// another. It goes through `Backends::kokoro` rather than the trait,
/// because listing voices is not something every backend can do
/// (docs/design.md#voice-contract). A fully cached project needs no server,
/// so it is not asked.
async fn check_voices(
    backends: &Backends,
    backend_id: &str,
    compiled: &teleprompt_compile::CompileOutput,
    cache: &VoiceCache,
) -> Result<(), DubError> {
    // A corrupt entry counts as missing: it will be re-rendered.
    let anything_to_synthesize = compiled.narration.iter().any(|detail| {
        !matches!(
            cache.lookup_meta(&detail.cache_key),
            Ok(teleprompt_cache::CacheRead::Hit(_))
        )
    });
    let Some(kokoro) = backends
        .kokoro(backend_id)
        .filter(|_| anything_to_synthesize)
    else {
        return Ok(());
    };
    let wanted = compiled
        .narration
        .iter()
        .filter_map(|d| d.synth_request.voice.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if wanted.is_empty() {
        return Ok(());
    }
    // A server that fails to answer is a runtime failure (exit 1,
    // docs/design.md#backend-failure; the error names the URL). Only one
    // that answers without the voice is a script problem (exit 2).
    let available = kokoro
        .voices()
        .await
        .map_err(|e| DubError::Runtime(e.to_string()))?;
    let problems: Vec<String> = wanted
        .iter()
        .filter(|v| !available.contains(v))
        .map(|v| {
            format!(
                "voice `{v}` is not available on the kokoro server at {} \
                 (available: {})",
                kokoro.base_url(),
                available.join(", ")
            )
        })
        .collect();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(DubError::Validation(problems))
    }
}

/// Every line's audio, in document order, rendering at most `limit` keys at
/// once.
///
/// Bounded: the model server is the bottleneck, and fanning out wider than
/// it serves makes the run slower and its failures worse. At 1, with tasks
/// spawned in document order on the CLI's current-thread runtime, the run is
/// serial in every observable way, stderr included. The first error aborts
/// every task in flight; entries already stored stay, since the cache is
/// content-addressed.
async fn render_all(
    backend: &Arc<dyn VoiceBackend>,
    cache: &Arc<VoiceCache>,
    narration: &[NarrationDetail],
    limit: usize,
) -> Result<Vec<Rendered>, DubError> {
    let permits = Arc::new(tokio::sync::Semaphore::new(limit));
    // Progress is per line, in completion order with a running count: under
    // concurrency, which line finishes next is a fact about the server, and
    // relabelling it into document order would misreport what happened.
    let total = narration.len();
    let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut tasks: tokio::task::JoinSet<(Vec<usize>, Result<RenderedAudio, DubError>)> =
        tokio::task::JoinSet::new();
    for indices in groups_by_key(narration) {
        let detail = narration[indices[0]].clone();
        let line_ids: Vec<String> = indices
            .iter()
            .map(|&i| narration[i].line_id.clone())
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
                // Progress counts the script's lines, not requests.
                for id in &line_ids {
                    let n = completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    eprintln!("  [{n}/{total}] {id} done");
                }
            }
            (indices, r)
        });
    }

    // Results arrive in completion order, so each is filed at its indices.
    let mut slots: Vec<Option<Rendered>> = (0..total).map(|_| None).collect();
    while let Some(joined) = tasks.join_next().await {
        let (indices, r) = joined.expect("a render task panicked");
        let audio = match r {
            Ok(audio) => audio,
            Err(e) => {
                tasks.abort_all();
                return Err(e);
            }
        };
        for i in indices {
            slots[i] = Some(Rendered {
                line_id: narration[i].line_id.clone(),
                wav_bytes: audio.wav_bytes.clone(),
                rendered_ms: audio.rendered_ms,
                sample_rate: audio.sample_rate,
                channels: audio.channels,
                cache_warning: audio.cache_warning.clone(),
            });
        }
    }
    Ok(slots
        .into_iter()
        .map(|r| r.expect("every index rendered when there was no failure"))
        .collect())
}

/// Line indices grouped by cache key, one group per task (see
/// [`RenderedAudio`]), ordered by first occurrence so tasks spawn in
/// document order and stderr is reproducible.
fn groups_by_key(narration: &[NarrationDetail]) -> Vec<Vec<usize>> {
    let mut groups: std::collections::HashMap<teleprompt_cache::CacheKey, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, detail) in narration.iter().enumerate() {
        groups.entry(detail.cache_key.clone()).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = groups.into_values().collect();
    groups.sort_by_key(|indices| indices[0]);
    groups
}

/// The rendered audio, in document order, with what the manifest needs
/// from it.
struct Audio {
    /// Each line's id, WAV bytes and rendered length.
    lines: Vec<(String, Vec<u8>, u64)>,
    /// The first line's rate and channels; the manifest has one `AudioInfo`
    /// per locale, and the first line in document order does not depend on
    /// which request answered first.
    format: Option<(u32, u16)>,
    /// Collected here because the recompile sees a healed cache.
    cache_warnings: Vec<String>,
}

impl Audio {
    fn collect(rendered: Vec<Rendered>) -> Self {
        let mut audio = Audio {
            lines: Vec::with_capacity(rendered.len()),
            format: None,
            cache_warnings: Vec::new(),
        };
        for r in rendered {
            if let Some(w) = &r.cache_warning {
                audio
                    .cache_warnings
                    .push(format!("line `{}`: {w}", r.line_id));
            }
            audio.format.get_or_insert((r.sample_rate, r.channels));
            audio.lines.push((r.line_id, r.wav_bytes, r.rendered_ms));
        }
        audio
    }

    /// Every rendered length against the (measured) timeline, before
    /// anything is written: an output directory that disagrees with its own
    /// manifest is worse than none. It should never fire, since `store`
    /// returns the entry the recompile reads; it catches a key or metadata
    /// drift that would make the recompile resolve a different entry.
    fn check_lengths(&self, timeline: &teleprompt_schedule::Timeline) -> Result<(), DubError> {
        for (line_id, _, rendered_ms) in &self.lines {
            if let Some(published_ms) = published_duration_ms(timeline, line_id) {
                length_mismatch(line_id, *rendered_ms, published_ms)
                    .map_or(Ok(()), |m| Err(DubError::Runtime(m)))?;
            }
        }
        Ok(())
    }

    /// The manifest, with each line's `audio_hash` the hash of its WAV bytes.
    ///
    /// `manifest::build` seeds `audio_hash` with the timeline's hash of the
    /// cache key; the manifest publishes the bytes' hash
    /// (docs/design.md#manifest), and only `dub` has them. Done before
    /// `--check` so a comparison is like-for-like.
    fn manifest(&self, compiled: &teleprompt_compile::CompileOutput) -> NarrationManifest {
        let (sample_rate, channels) = self.format.unwrap_or((NO_AUDIO_SAMPLE_RATE, 1));
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
        for (line_id, bytes, _) in &self.lines {
            if let Some(seg) = built.lines.iter_mut().find(|s| s.id == *line_id) {
                seg.audio_hash = Hash::of(bytes);
            }
        }
        built
    }
}

/// How `built` differs from the manifest committed under `out_root`. No
/// manifest at all is maximal drift: everything is new.
fn drift_from_committed(
    out_root: &Path,
    locale: &str,
    built: &NarrationManifest,
) -> Result<ManifestDiff, DubError> {
    let committed = read_committed(&manifest_path(out_root, locale)).map_err(DubError::Runtime)?;
    Ok(match committed {
        Some(before) => manifest_diff::diff(&before, built),
        None => manifest_diff::diff(
            &NarrationManifest {
                lines: Vec::new(),
                chapters: Vec::new(),
                duration_ms: 0,
                ..built.clone()
            },
            built,
        ),
    })
}

/// Writes each line's WAV and then the manifest, returning the paths
/// written.
fn write_output(
    out_root: &Path,
    locale: &str,
    audio: &Audio,
    built: &NarrationManifest,
) -> Result<Vec<PathBuf>, DubError> {
    let dir = locale_dir(out_root, locale);
    let audio_dir = dir.join("audio");
    std::fs::create_dir_all(&audio_dir)
        .map_err(|e| DubError::Runtime(format!("cannot create {}: {e}", audio_dir.display())))?;

    let mut written = Vec::new();
    for (line_id, bytes, _) in &audio.lines {
        let path = dir.join(manifest::audio_path(line_id, "wav"));
        std::fs::write(&path, bytes)
            .map_err(|e| DubError::Runtime(format!("cannot write {}: {e}", path.display())))?;
        written.push(path);
    }

    let path = manifest_path(out_root, locale);
    let json = serde_json::to_string_pretty(built)
        .map_err(|e| DubError::Runtime(format!("cannot serialize manifest: {e}")))?;
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| DubError::Runtime(format!("cannot write {}: {e}", path.display())))?;
    written.push(path);
    Ok(written)
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

    /// A manifest publishing 3250ms beside a 6500ms file: the guard names
    /// the line and both numbers.
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
