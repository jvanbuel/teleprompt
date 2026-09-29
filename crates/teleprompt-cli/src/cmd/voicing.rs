//! How each line of a prompted script sounds when its voice reads it: the
//! prompter's view of what `dub` would make. A line with a current take is
//! read from it; any other is synthesized when first asked for, into the
//! voice cache `dub` and `build` read from.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use teleprompt_cache::VoiceCache;
use teleprompt_compile::NarrationDetail;
use teleprompt_voice::VoiceBackend;

use crate::project::Project;

/// A script's voice, compiled afresh each time it is described, since a
/// line synthesized or edited since changes what it says.
pub struct Voicing {
    project: Project,
    script: PathBuf,
    locale: String,
    /// The last compile: each line's request and take, and who reads.
    voiced: Mutex<Option<Voiced>>,
    runtime: OnceLock<Result<tokio::runtime::Runtime, String>>,
}

#[derive(Clone)]
struct Voiced {
    backend: Arc<dyn VoiceBackend>,
    lines: Vec<NarrationDetail>,
    length_ms: u64,
}

impl Voicing {
    pub fn new(project: &Project, script: &std::path::Path, locale: &str) -> Self {
        Self {
            project: project.clone(),
            script: script.to_path_buf(),
            locale: locale.to_string(),
            voiced: Mutex::new(None),
            runtime: OnceLock::new(),
        }
    }

    fn cache(&self) -> VoiceCache {
        VoiceCache::new(self.project.caches().root)
    }

    /// The script compiled as it now reads, or as it last compiled.
    fn refresh(&self) -> Option<Voiced> {
        let mut voiced = self.voiced.lock().unwrap_or_else(PoisonError::into_inner);
        if let Ok((compiled, backend)) =
            crate::cmd::check::compile_script(&self.project, &self.script, &self.locale)
        {
            *voiced = Some(Voiced {
                backend,
                length_ms: compiled.timeline.duration_ms.ms(),
                lines: compiled.narration,
            });
        }
        voiced.clone()
    }

    /// Who reads the script, how long it runs, and each line's audio by
    /// id, for the script the prompter serves.
    pub fn describe(&self) -> Option<Description> {
        let voiced = self.refresh()?;
        let voice = voiced
            .lines
            .iter()
            .find_map(|l| l.synth_request.voice.clone());
        let name = match voice {
            Some(voice) => format!("{} · {voice}", voiced.backend.id()),
            None => voiced.backend.id().to_string(),
        };
        let cache = self.cache();
        let lines = voiced
            .lines
            .iter()
            .map(|l| (l.line_id.to_string(), line_audio(l, &cache)))
            .collect();
        Some(Description {
            name,
            length_ms: voiced.length_ms,
            lines,
        })
    }

    /// Line `id`'s audio as a WAV: its take, or its voice's, made now if
    /// the cache lacks it or `fresh` asks for it anew. `None` for a line
    /// the script does not have.
    pub fn audio(&self, id: &str, fresh: bool) -> Result<Option<Vec<u8>>, String> {
        let known = self
            .voiced
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(voiced) = known.or_else(|| self.refresh()) else {
            return Ok(None);
        };
        let Some(line) = voiced.lines.iter().find(|l| l.line_id == id) else {
            return Ok(None);
        };
        if line.take.is_some() {
            let take = self.project.takes_dir().join(format!("{id}.wav"));
            return std::fs::read(&take)
                .map(Some)
                .map_err(|e| format!("cannot read {}: {e}", take.display()));
        }
        let cache = self.cache();
        if fresh {
            cache.forget(&line.cache_key).map_err(|e| e.to_string())?;
        } else if let Some(hit) = cache
            .lookup(&line.cache_key)
            .map_err(|e| e.to_string())?
            .hit()
        {
            return Ok(Some(hit.wav));
        }
        let runtime = self
            .runtime
            .get_or_init(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                    .map_err(|e| format!("cannot start the voice's runtime: {e}"))
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let stored = runtime.block_on(crate::cmd::dub::synthesize_and_store(
            &voiced.backend,
            &cache,
            line,
        ))?;
        Ok(Some(stored.wav))
    }
}

/// [`Voicing::describe`]'s answer.
pub struct Description {
    pub name: String,
    pub length_ms: u64,
    pub lines: Vec<(String, serde_json::Value)>,
}

/// Where a line's audio comes from, whether it is made yet, how long it
/// runs and when each of its words starts.
fn line_audio(line: &NarrationDetail, cache: &VoiceCache) -> serde_json::Value {
    let url = format!("/api/v1/voice/{}.wav", line.line_id);
    let instruct = line.synth_request.instruct.clone();
    if let Some(take) = &line.take {
        let words = teleprompt_prompter::word_starts(&line.text, take.duration_ms);
        return serde_json::json!({
            "audio": { "source": "take", "url": url, "ready": true,
                       "duration_ms": take.duration_ms, "words": words },
            "instruct": instruct,
        });
    }
    let meta = cache
        .lookup_meta(&line.cache_key)
        .ok()
        .and_then(|m| m.hit());
    let (duration, words) = match meta {
        Some(meta) => {
            let count = line.text.split_whitespace().count();
            // The voice's own timings where they are one per word, as the
            // script counts words; spread over the line otherwise.
            let words = match meta.word_timings {
                Some(t) if t.len() == count => t.iter().map(|w| w.start_ms).collect(),
                _ => teleprompt_prompter::word_starts(&line.text, meta.duration_ms),
            };
            (Some(meta.duration_ms), Some(words))
        }
        None => (None, None),
    };
    serde_json::json!({
        "audio": { "source": "voice", "url": url, "ready": duration.is_some(),
                   "duration_ms": duration, "words": words },
        "instruct": instruct,
    })
}
