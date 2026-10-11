//! Where each line's audio comes from: its current take, else the voice
//! cache, else its backend, stored in the cache as it is made. `dub` asks
//! for every line, the prompter for one; both get the same answer.

use std::sync::Arc;

use teleprompt_core::{Progress, Reporter};
use teleprompt_pipeline::compile::NarrationDetail;
use teleprompt_voice::backends::Cast;
use teleprompt_voice::cache::{CachedAudio, VoiceCache};
use teleprompt_voice::takes::Takes;
use teleprompt_voice::{wav, VoiceBackend};

use crate::dub::publish::LineAudio;
use crate::project::Project;
use crate::Failure;

/// A compile's lines, voiced: its cast, the voice cache and the takes.
pub struct Lines {
    cast: Cast,
    cache: Arc<VoiceCache>,
    takes: Takes,
}

impl Lines {
    pub fn new(cast: Cast, cache: VoiceCache, takes: Takes) -> Self {
        Self {
            cast,
            cache: Arc::new(cache),
            takes,
        }
    }

    /// Spoken by `cast`, from `project`'s voice cache and takes.
    pub fn of(project: &Project, cast: Cast) -> Result<Self, Failure> {
        let takes =
            Takes::load(&project.takes_dir()).map_err(|e| Failure::Runtime(e.to_string()))?;
        Ok(Self::new(
            cast,
            VoiceCache::new(project.caches().root),
            takes,
        ))
    }

    pub fn cast(&self) -> &Cast {
        &self.cast
    }

    /// `detail`'s audio in its own format: its take, else the cached audio,
    /// else synthesized and stored. `fresh` drops the cached audio first.
    pub async fn line(&self, detail: &NarrationDetail, fresh: bool) -> Result<LineAudio, Failure> {
        if detail.take.is_some() {
            return self.take(detail);
        }
        if fresh {
            self.cache
                .forget(&detail.cache_key)
                .map_err(|e| failed(detail, &e))?;
        }
        let backend = self.backend(detail)?;
        render_one(&backend, &self.cache, detail).await
    }

    /// Every line's audio in document order, each in its own format, with
    /// at most `limit` keys rendered at once.
    ///
    /// Before anything is synthesized, the voices the lines ask for are
    /// checked against each backend that lists its own.
    pub async fn all(
        &self,
        narration: &[NarrationDetail],
        limit: usize,
        reporter: &dyn Reporter,
    ) -> Result<Vec<LineAudio>, Failure> {
        for id in self.cast.ids() {
            check_voices(&self.cast, id, narration, &self.cache).await?;
        }
        let synthesized: Vec<NarrationDetail> = narration
            .iter()
            .filter(|d| d.take.is_none())
            .cloned()
            .collect();
        let mut synthesized = self
            .render_all(&synthesized, limit, reporter)
            .await?
            .into_iter();
        narration
            .iter()
            .map(|detail| match detail.take {
                Some(_) => self.take(detail),
                None => Ok(synthesized
                    .next()
                    .expect("BUG: render_all returns one audio per line it is given")),
            })
            .collect()
    }

    /// `detail`'s take as recorded, refused if it has changed since.
    fn take(&self, detail: &NarrationDetail) -> Result<LineAudio, Failure> {
        let bytes = self
            .takes
            .read(detail.line_id.as_str())
            .map_err(|e| failed(detail, &e))?;
        let pcm = wav::decode(&bytes).map_err(|e| failed(detail, &e))?;
        Ok(LineAudio {
            line_id: detail.line_id.clone(),
            duration_ms: pcm.duration_ms(),
            sample_rate: pcm.sample_rate,
            channels: pcm.channels,
            wav: bytes,
            cache_warning: None,
        })
    }

    fn backend(&self, detail: &NarrationDetail) -> Result<Arc<dyn VoiceBackend>, Failure> {
        self.cast.get(&detail.backend).cloned().ok_or_else(|| {
            Failure::Runtime(format!(
                "line `{}`: no voice backend `{}`",
                detail.line_id, detail.backend
            ))
        })
    }

    /// Every line's audio, in document order, rendering at most `limit`
    /// keys at once.
    ///
    /// Bounded: the model server is the bottleneck, and fanning out wider
    /// than it serves makes the run slower and its failures worse. At 1,
    /// with tasks spawned in document order on the CLI's current-thread
    /// runtime, the run is serial in every observable way, stderr included.
    /// The first error aborts every task in flight; entries already stored
    /// stay, since the cache is content-addressed.
    async fn render_all(
        &self,
        narration: &[NarrationDetail],
        limit: usize,
        reporter: &dyn Reporter,
    ) -> Result<Vec<LineAudio>, Failure> {
        let permits = Arc::new(tokio::sync::Semaphore::new(limit));
        // Progress is per line, in completion order with a running count:
        // under concurrency, which line finishes next is a fact about the
        // server, and relabelling it into document order would misreport
        // what happened.
        let total = narration.len();
        let mut completed = 0;

        let mut tasks: tokio::task::JoinSet<(Vec<usize>, Result<LineAudio, Failure>)> =
            tokio::task::JoinSet::new();
        for indices in groups_by_key(narration) {
            let detail = narration[indices[0]].clone();
            let backend = self.backend(&detail)?;
            let cache = self.cache.clone();
            let permits = permits.clone();
            tasks.spawn(async move {
                let _permit = permits
                    .acquire_owned()
                    .await
                    .expect("BUG: the semaphore is never closed, only dropped with this function");
                let r = render_one(&backend, &cache, &detail).await;
                (indices, r)
            });
        }

        // Results arrive in completion order, so each is filed at its indices.
        let mut slots: Vec<Option<LineAudio>> = (0..total).map(|_| None).collect();
        while let Some(joined) = tasks.join_next().await {
            let (indices, r) = joined
                .map_err(|e| Failure::Runtime(format!("a render task did not finish: {e}")))?;
            let audio = match r {
                Ok(audio) => audio,
                Err(e) => {
                    tasks.abort_all();
                    return Err(e);
                }
            };
            for i in indices {
                let line_id = narration[i].line_id.clone();
                // Progress counts the script's lines, not requests.
                completed += 1;
                reporter.progress(Progress::Voice {
                    done: completed,
                    of: total,
                    line: line_id.clone(),
                });
                slots[i] = Some(LineAudio {
                    line_id,
                    ..audio.clone()
                });
            }
        }
        Ok(slots
            .into_iter()
            .map(|r| r.expect("BUG: every index is filed once the tasks drain without a failure"))
            .collect())
    }
}

fn failed(detail: &NarrationDetail, e: &dyn std::fmt::Display) -> Failure {
    Failure::Runtime(format!("line `{}`: {e}", detail.line_id))
}

/// Synthesizes `detail`'s line and stores it, returning the entry the cache
/// holds; an error names the line.
///
/// The request is the one `compile` measured, not one rebuilt here, so the
/// audio and the published duration share one source of truth. Another
/// process may store the same key concurrently; `VoiceCache::store` returns
/// whichever entry won, so the caller's bytes and the sidecar a recompile
/// reads are the same entry.
async fn synthesize_and_store(
    backend: &Arc<dyn VoiceBackend>,
    cache: &VoiceCache,
    detail: &NarrationDetail,
) -> Result<CachedAudio, Failure> {
    let synthesized = backend
        .synthesize(&detail.synth_request)
        .await
        .map_err(|e| failed(detail, &e))?;
    cache
        .store(
            &detail.cache_key,
            &synthesized.pcm,
            synthesized.word_timings.as_deref(),
        )
        .map_err(|e| failed(detail, &e))
}

/// One cache key's audio, from the cache or, on a miss, from `backend`
/// and then stored. `detail` is any line that resolves to the key: equal
/// keys imply identical `SynthRequest`s, and the key hashes the text, not
/// the line id (docs/design.md#voice-cache).
async fn render_one(
    backend: &Arc<dyn VoiceBackend>,
    cache: &VoiceCache,
    detail: &NarrationDetail,
) -> Result<LineAudio, Failure> {
    let read = cache
        .lookup(&detail.cache_key)
        .map_err(|e| failed(detail, &e))?;
    let cache_warning = read.warning();
    let audio = match read.hit() {
        Some(cached) => cached,
        None => synthesize_and_store(backend, cache, detail).await?,
    };
    Ok(LineAudio {
        line_id: detail.line_id.clone(),
        wav: audio.wav,
        duration_ms: audio.duration_ms,
        sample_rate: audio.sample_rate,
        channels: audio.channels,
        cache_warning,
    })
}

/// Line indices grouped by cache key, one group per task, so a key is
/// rendered once and this run never races itself in `store`. Ordered by
/// first occurrence so tasks spawn in document order and stderr is
/// reproducible.
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

/// Checks the voices the script asks for against the server, once, before
/// anything is synthesized.
///
/// Here, not in `check`: `check` stays offline, and a gate that depends on a
/// running server would pass a script on one machine and fail it on
/// another. Only a backend that lists its voices is checked
/// (docs/design.md#voice-contract). A fully cached project needs no server,
/// so it is not asked.
async fn check_voices(
    cast: &Cast,
    backend_id: &str,
    narration: &[NarrationDetail],
    cache: &VoiceCache,
) -> Result<(), Failure> {
    // A corrupt entry counts as missing: it will be re-rendered.
    let anything_to_synthesize = narration.iter().any(|detail| {
        detail.backend == backend_id
            && detail.take.is_none()
            && !matches!(
                cache.lookup_meta(&detail.cache_key),
                Ok(teleprompt_voice::cache::CacheRead::Hit(_))
            )
    });
    let Some(backend) = cast.get(backend_id).filter(|_| anything_to_synthesize) else {
        return Ok(());
    };
    let wanted = narration
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
        Err(Failure::Validation(teleprompt_core::Diagnostics(
            problems
                .into_iter()
                .map(teleprompt_core::Diagnostic::error)
                .collect(),
        )))
    }
}
