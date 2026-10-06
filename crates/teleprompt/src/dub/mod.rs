use std::path::{Path, PathBuf};
use std::sync::Arc;

use publish::{LineAudio, Published};
use teleprompt_compile::NarrationDetail;
use teleprompt_core::config::OutputConfig;
use teleprompt_core::{LineId, SpanMs};
use teleprompt_manifest::diff::{self as manifest_diff, ManifestDiff};
use teleprompt_manifest::{audio_path, NarrationManifest, MANIFEST_VERSION};
use teleprompt_manifest::{captions, chapters};
use teleprompt_voice::cache::{CachedAudio, VoiceCache};
use teleprompt_voice::takes::Takes;
use teleprompt_voice::VoiceBackend;

use crate::project::{CacheDir, Clips, Compiled, Script};
use crate::voice::Backends;
use crate::Failure;

pub(crate) mod publish;

/// A script dubbed, in state `S`: [`Written`] under an output root, or
/// [`Compared`] with what is there. What only one state has is only in
/// that state, so nothing asks a check for the files it wrote, or
/// captures from a manifest that was never published.
///
/// ```
/// # use teleprompt::{capture::Scenes, dub::Dubber, project::Script};
/// # async fn f(script: &Script, scenes: &Scenes<'_>) {
/// let dubbed = Dubber::new(script).dub("out".as_ref()).await.unwrap();
/// scenes.capture(&dubbed, &mut |_| {});
/// # }
/// ```
///
/// A check publishes nothing, so there is nothing to capture from:
///
/// ```compile_fail
/// # use teleprompt::{capture::Scenes, dub::Dubber, project::Script};
/// # async fn f(script: &Script, scenes: &Scenes<'_>) {
/// let checked = Dubber::new(script).check("out".as_ref()).await.unwrap();
/// scenes.capture(&checked, &mut |_| {});
/// # }
/// ```
pub struct Dubbed<S> {
    pub manifest: NarrationManifest,
    /// Every shot's source, which a capture backend runs. The manifest
    /// names the shots; only this says what they do.
    pub shots: std::collections::BTreeMap<teleprompt_core::ShotId, teleprompt_compile::ShotSource>,
    /// The scenes as configured, which a capture backend opens.
    pub scenes: std::collections::BTreeMap<String, teleprompt_core::config::SceneConfig>,
    /// The script's `output:` block, which `build` needs and the manifest
    /// does not carry.
    pub output: OutputConfig,
    pub warnings: Vec<String>,
    pub state: S,
}

/// Published: the files written under the output root.
pub struct Written {
    pub files: Vec<PathBuf>,
}

/// Compared with the manifest already under the output root, writing
/// nothing there.
pub struct Compared {
    pub drift: ManifestDiff,
}

impl<S> Dubbed<S> {
    fn new(voiced: Voiced, state: S) -> Self {
        Dubbed {
            manifest: voiced.published.manifest,
            shots: voiced.compiled.shots,
            scenes: voiced.compiled.scenes,
            output: voiced.compiled.output,
            warnings: voiced.warnings,
            state,
        }
    }
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
/// one render. [`voice`] groups lines by key so each key is rendered
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
) -> Result<RenderedAudio, Failure> {
    let read = cache
        .lookup(&detail.cache_key)
        .map_err(|e| Failure::Runtime(format!("line `{}`: {e}", detail.line_id)))?;
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
                .map_err(Failure::Runtime)?;
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

/// Voices a script and publishes it under an output root:
/// `<root>/<locale>/narration.json` and its audio.
pub struct Dubber<'a> {
    script: &'a Script,
}

impl<'a> Dubber<'a> {
    pub fn new(script: &'a Script) -> Self {
        Self { script }
    }

    /// Voices the script and writes it under `out_root`.
    pub async fn dub(&self, out_root: &Path) -> Result<Dubbed<Written>, Failure> {
        let voiced = voice(self.script).await?;
        let files = write_output(out_root, self.script.locale(), &voiced.published)?;
        Ok(Dubbed::new(voiced, Written { files }))
    }

    /// Voices the script and compares it with the manifest under
    /// `out_root`, writing nothing there.
    pub async fn check(&self, out_root: &Path) -> Result<Dubbed<Compared>, Failure> {
        let voiced = voice(self.script).await?;
        let drift =
            drift_from_committed(out_root, self.script.locale(), &voiced.published.manifest)?;
        Ok(Dubbed::new(voiced, Compared { drift }))
    }
}

/// A script voiced: every line's audio made or read from its take, and
/// the manifest that publishes it.
pub(crate) struct Voiced {
    pub compiled: teleprompt_compile::CompileOutput,
    pub published: Published,
    pub warnings: Vec<String>,
}

/// Voices the script, what `dub` does short of writing it out, and what
/// the prompter plays.
pub(crate) async fn voice(script: &Script) -> Result<Voiced, Failure> {
    let (project, backends) = (script.project(), script.backends());
    // Synthesize only with the backend the keys were computed from; see
    // `Script::compile`.
    let Compiled {
        output: compiled,
        backend,
        ..
    } = script.compile().map_err(Failure::Validation)?;
    let cache = Arc::new(VoiceCache::new(project.caches().root));
    let voices = backends
        .voices(&backend, &compiled.narration)
        .map_err(Failure::Runtime)?;

    for id in voices.keys() {
        check_voices(backends, id, &compiled, &cache).await?;
    }
    let limit = backends.concurrency(backend.id());
    let takes = Takes::load(&project.takes_dir()).map_err(|e| Failure::Runtime(e.to_string()))?;
    let synthesized: Vec<NarrationDetail> = compiled
        .narration
        .iter()
        .filter(|d| d.take.is_none())
        .cloned()
        .collect();
    let rendered = render_all(&voices, &cache, &synthesized, limit).await?;
    // The re-render notices come first: they explain why anything below them
    // is being recomputed at all.
    let mut warnings: Vec<String> = rendered
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
        publish::with_takes(&compiled.narration, synthesized, &takes).map_err(Failure::Runtime)?;

    // Recompile against the now-warm cache: the first compile ran before
    // anything was rendered, so on a cold project it holds estimates. This
    // makes `dub` idempotent, and costs no synthesis.
    let compiled = script.compile().map_err(Failure::Validation)?.output;
    let published = publish::publish(&compiled, audio).map_err(Failure::Runtime)?;
    warnings.extend(compiled.warnings.iter().cloned());
    Ok(Voiced {
        compiled,
        published,
        warnings,
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
) -> Result<(), Failure> {
    // A corrupt entry counts as missing: it will be re-rendered.
    let anything_to_synthesize = compiled.narration.iter().any(|detail| {
        detail.backend == backend_id
            && detail.take.is_none()
            && !matches!(
                cache.lookup_meta(&detail.cache_key),
                Ok(teleprompt_voice::cache::CacheRead::Hit(_))
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
    let available = available.map_err(|e| Failure::Runtime(e.to_string()))?;
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
        Err(Failure::Validation(problems))
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
) -> Result<Vec<Rendered>, Failure> {
    let permits = Arc::new(tokio::sync::Semaphore::new(limit));
    // Progress is per line, in completion order with a running count: under
    // concurrency, which line finishes next is a fact about the server, and
    // relabelling it into document order would misreport what happened.
    let total = narration.len();
    let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut tasks: tokio::task::JoinSet<(Vec<usize>, Result<RenderedAudio, Failure>)> =
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
                    crate::progress::progress(
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
    let mut groups: std::collections::HashMap<teleprompt_voice::cache::CacheKey, Vec<usize>> =
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
) -> Result<ManifestDiff, Failure> {
    let committed = read_committed(&manifest_path(out_root, locale)).map_err(Failure::Runtime)?;
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
) -> Result<Vec<PathBuf>, Failure> {
    let built = &published.manifest;
    let dir = locale_dir(out_root, locale);
    let audio_dir = dir.join("audio");
    std::fs::create_dir_all(&audio_dir)
        .map_err(|e| Failure::Runtime(format!("cannot create {}: {e}", audio_dir.display())))?;

    let mut written = Vec::new();
    for (line_id, bytes) in &published.lines {
        let path = dir.join(audio_path(line_id, "wav"));
        std::fs::write(&path, bytes)
            .map_err(|e| Failure::Runtime(format!("cannot write {}: {e}", path.display())))?;
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
            .map_err(|e| Failure::Runtime(format!("cannot write {}: {e}", path.display())))?;
        written.push(path);
    }

    let path = manifest_path(out_root, locale);
    let json = serde_json::to_string_pretty(built)
        .map_err(|e| Failure::Runtime(format!("cannot serialize manifest: {e}")))?;
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| Failure::Runtime(format!("cannot write {}: {e}", path.display())))?;
    written.push(path);
    Ok(written)
}

pub fn render_dub(out: &Dubbed<Written>) -> String {
    let mut s = format!(
        "{} ({}) — {} line(s), {:.1}s\n",
        out.manifest.script,
        out.manifest.locale,
        out.manifest.lines.len(),
        out.manifest.duration_ms.ms() as f64 / 1000.0,
    );
    for path in &out.state.files {
        s.push_str(&format!("  wrote {}\n", path.display()));
    }
    s
}

/// `dub --clips`, once `script` is dubbed into `out_root`: records the
/// shots not yet captured as `capture` does, then puts every shot's clip in
/// `<locale>/clips/`. A shot nothing here can record is left out, with
/// capture's warning.
pub fn add_clips(
    script: &Script,
    out_root: &Path,
    dubbed: &mut Dubbed<Written>,
) -> Result<(), Failure> {
    let clips_dir = script.project().caches().clips();
    let frame = crate::build::FrameOverride::default().frame(&dubbed.output);
    let scenes = crate::capture::Scenes::new(script.project().registry.scenes, &clips_dir, frame);
    let captured = scenes.capture(dubbed, &mut crate::progress::capture_progress);
    dubbed.warnings.extend(captured.warnings);
    let into = locale_dir(out_root, script.locale()).join("clips");
    let placed = place_clips(&dubbed.manifest, &clips_dir, &into)
        .map_err(|e| Failure::Runtime(format!("{}: {e}", into.display())))?;
    dubbed.state.files.extend(placed);
    Ok(())
}

/// Every clip `manifest` names that `cache` holds, in `into`, and nothing
/// else there. Hard links where the file system allows, so a package costs
/// no space; copies where it does not.
fn place_clips(
    manifest: &NarrationManifest,
    cache: &CacheDir<Clips>,
    into: &Path,
) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(into)?;
    let named: std::collections::BTreeSet<String> = manifest
        .shots
        .iter()
        .filter_map(|s| s.clip_name())
        .collect();
    for entry in std::fs::read_dir(into)? {
        let entry = entry?;
        if !named.contains(&*entry.file_name().to_string_lossy()) {
            std::fs::remove_file(entry.path())?;
        }
    }
    let mut placed = Vec::new();
    for name in &named {
        let (from, to) = (cache.join(name), into.join(name));
        if !from.is_file() {
            continue;
        }
        if to.exists() {
            std::fs::remove_file(&to)?;
        }
        if std::fs::hard_link(&from, &to).is_err() {
            let partial = into.join(format!("{name}.partial"));
            std::fs::copy(&from, &partial)?;
            std::fs::rename(&partial, &to)?;
        }
        placed.push(to);
    }
    Ok(placed)
}
