use std::path::{Path, PathBuf};
use std::sync::Arc;

use teleprompt_cache::{CachedAudio, VoiceCache};
use teleprompt_compile::publish::{self, LineAudio};
use teleprompt_compile::NarrationDetail;
use teleprompt_core::config::OutputConfig;
use teleprompt_core::{LineId, SpanMs};
use teleprompt_manifest::diff::{self as manifest_diff, ManifestDiff};
use teleprompt_manifest::{audio_path, NarrationManifest, MANIFEST_VERSION};
use teleprompt_manifest::{captions, chapters};
use teleprompt_plugin::voice::VoiceBackend;
use teleprompt_voice::takes::Takes;

use crate::project::Project;
use crate::voice::Backends;

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
    line_id: LineId,
    audio: LineAudio,
    cache_warning: Option<String>,
}

/// Synthesizes `detail`'s line and stores it, returning the entry the cache
/// holds; an error names the line.
///
/// The request is the one `compile` measured, not one rebuilt here, so the
/// audio and the published duration share one source of truth. Another
/// process may store the same key concurrently; `VoiceCache::store` returns
/// whichever entry won, so the caller's bytes and the sidecar a recompile
/// reads are the same entry.
pub(crate) async fn synthesize_and_store(
    backend: &Arc<dyn VoiceBackend>,
    cache: &VoiceCache,
    detail: &NarrationDetail,
) -> Result<CachedAudio, String> {
    let line = |e: &dyn std::fmt::Display| format!("line `{}`: {e}", detail.line_id);
    let synthesized = backend
        .synthesize(&detail.synth_request)
        .await
        .map_err(|e| line(&e))?;
    cache
        .store(
            &detail.cache_key,
            &synthesized.pcm,
            synthesized.word_timings.as_deref(),
        )
        .map_err(|e| line(&e))
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
            let stored = synthesize_and_store(backend, cache, detail)
                .await
                .map_err(DubError::Runtime)?;
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
    // does not reach construction, as in `Project::compile`.
    let backends = project.backends();
    run_dub_with(&backends, project, script, locale, out_root, check_only).await
}

/// [`run_dub`] against caller-supplied backends; the seam exists for the
/// reason `Project::compile_with` gives.
pub async fn run_dub_with(
    backends: &Backends,
    project: &Project,
    script: &Path,
    locale: &str,
    out_root: &Path,
    check_only: bool,
) -> Result<DubOutput, DubError> {
    // Synthesize only with the backend the keys were computed from; see
    // `Project::compile_with`.
    let (compiled, backend) = project
        .compile_with(backends, script, locale)
        .map_err(DubError::Validation)?;
    let cache = Arc::new(VoiceCache::new(project.caches().root));
    let voices = backends
        .voices(&backend, &compiled.narration)
        .map_err(DubError::Runtime)?;

    for id in voices.keys() {
        check_voices(backends, id, &compiled, &cache).await?;
    }
    let limit = backends.concurrency(backend.id());
    let takes = Takes::load(&project.takes_dir()).map_err(|e| DubError::Runtime(e.to_string()))?;
    let synthesized: Vec<NarrationDetail> = compiled
        .narration
        .iter()
        .filter(|d| d.take.is_none())
        .cloned()
        .collect();
    let rendered = render_all(&voices, &cache, &synthesized, limit).await?;
    let cache_warnings: Vec<String> = rendered
        .iter()
        .filter_map(|r| {
            Some(format!(
                "line `{}`: {}",
                r.line_id,
                r.cache_warning.as_ref()?
            ))
        })
        .collect();
    let synthesized = rendered.into_iter().map(|r| r.audio).collect();
    let audio =
        publish::with_takes(&compiled.narration, synthesized, &takes).map_err(DubError::Runtime)?;

    // Recompile against the now-warm cache: the first compile ran before
    // anything was rendered, so on a cold project it holds estimates. This
    // makes `dub` idempotent, and costs no synthesis.
    let (compiled, _) = project
        .compile_with(backends, script, locale)
        .map_err(DubError::Validation)?;
    let published = publish::publish(&compiled, audio).map_err(DubError::Runtime)?;
    let built = published.manifest.clone();

    // The re-render notices come first: they explain why anything below them
    // is being recomputed at all.
    let mut warnings = cache_warnings;
    warnings.extend(compiled.warnings.iter().cloned());

    let (written, drift) = if check_only {
        (
            Vec::new(),
            Some(drift_from_committed(out_root, locale, &built)?),
        )
    } else {
        (write_output(out_root, locale, &published)?, None)
    };
    Ok(DubOutput {
        manifest: built,
        shots: compiled.shots.clone(),
        scenes: compiled.scenes.clone(),
        output: compiled.output.clone(),
        written,
        warnings,
        drift,
    })
}

/// Checks the voices the script asks for against the server, once, before
/// anything is synthesized.
///
/// Here, not in `check`: `check` stays offline, and a gate that depends on a
/// running server would pass a script on one machine and fail it on
/// another. Only a backend that lists its voices is checked
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
        detail.backend == backend_id
            && detail.take.is_none()
            && !matches!(
                cache.lookup_meta(&detail.cache_key),
                Ok(teleprompt_cache::CacheRead::Hit(_))
            )
    });
    let Some(backend) = backends
        .registry()
        .get(backend_id)
        .filter(|_| anything_to_synthesize)
    else {
        return Ok(());
    };
    let wanted = compiled
        .narration
        .iter()
        .filter(|d| d.backend == backend_id)
        .filter_map(|d| d.synth_request.voice.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if wanted.is_empty() {
        return Ok(());
    }
    // A server that fails to answer is a runtime failure (exit 1,
    // docs/design.md#backend-failure; the error names the URL). Only one
    // that answers without the voice is a script problem (exit 2).
    let Some(available) = backend.voices().await else {
        return Ok(());
    };
    let available = available.map_err(|e| DubError::Runtime(e.to_string()))?;
    let address = backend.address().unwrap_or_default();
    let problems: Vec<String> = wanted
        .iter()
        .filter(|v| !available.contains(v))
        .map(|v| {
            format!(
                "voice `{v}` is not available on the {backend_id} server at {address} \
                 (available: {})",
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
    voices: &crate::voice::Voices,
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
        let line_ids: Vec<LineId> = indices
            .iter()
            .map(|&i| narration[i].line_id.clone())
            .collect();
        let backend = voices[&detail.backend].clone();
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
                    crate::output::progress(
                        "voice",
                        || format!("  [{n}/{total}] {id} done"),
                        serde_json::json!({ "done": n, "of": total, "line": id }),
                    );
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
            let line_id = narration[i].line_id.clone();
            slots[i] = Some(Rendered {
                audio: LineAudio {
                    line_id: line_id.clone(),
                    wav: audio.wav_bytes.clone(),
                    duration_ms: audio.rendered_ms,
                    sample_rate: audio.sample_rate,
                    channels: audio.channels,
                },
                line_id,
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
                duration_ms: SpanMs::ZERO,
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
    published: &publish::Published,
) -> Result<Vec<PathBuf>, DubError> {
    let built = &published.manifest;
    let dir = locale_dir(out_root, locale);
    let audio_dir = dir.join("audio");
    std::fs::create_dir_all(&audio_dir)
        .map_err(|e| DubError::Runtime(format!("cannot create {}: {e}", audio_dir.display())))?;

    let mut written = Vec::new();
    for (line_id, bytes) in &published.lines {
        let path = dir.join(audio_path(line_id, "wav"));
        std::fs::write(&path, bytes)
            .map_err(|e| DubError::Runtime(format!("cannot write {}: {e}", path.display())))?;
        written.push(path);
    }

    // Beside the manifest, so a player serving the audio finds them too,
    // and chapters to paste into a video's description.
    let cues = captions::cues(built);
    for (name, text) in [
        ("captions.srt", captions::srt(&cues)),
        ("captions.vtt", captions::vtt(&cues)),
        ("chapters.txt", chapters::youtube(built).0),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, text)
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
        out.manifest.duration_ms.ms() as f64 / 1000.0,
    );
    for path in &out.written {
        s.push_str(&format!("  wrote {}\n", path.display()));
    }
    s
}

impl From<DubError> for crate::output::Outcome {
    fn from(e: DubError) -> Self {
        match e {
            DubError::Validation(errors) => Self::ValidationError(errors),
            DubError::Runtime(message) => Self::RuntimeFailure(message),
        }
    }
}

/// `dub`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub script: crate::cli::ScriptArgs,
    /// Output root; one self-contained directory is written per locale
    #[arg(long)]
    pub out: PathBuf,
    /// Compare against the manifest on disk; exit 3 on drift. Leaves
    /// `--out` untouched, but still synthesizes whatever is not already
    /// cached and writes it to the content-addressed cache — that is
    /// what the comparison measures against
    #[arg(long)]
    pub check: bool,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    use crate::output::Outcome;
    let project = args.script.project()?;
    let locale = args.script.locale(&project);
    let result = crate::cli::runtime()?.block_on(run_dub(
        &project,
        &args.script.script,
        &locale,
        &args.out,
        args.check,
    ))?;
    crate::cli::warn(&result.warnings);
    match &result.drift {
        Some(d) => crate::cli::emit_ok(format, d, &d.render(), d.is_empty()),
        None => crate::cli::emit_data(format, &result.manifest, &render_dub(&result)),
    }
    Ok(if result.drift.as_ref().is_some_and(|d| !d.is_empty()) {
        Outcome::Drift
    } else {
        Outcome::Ok
    })
}
