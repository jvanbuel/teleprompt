//! How each line of a prompted script sounds when its voice reads it: the
//! prompter's view of what `dub` would make. A line with a current take is
//! read from it; any other is synthesized when first asked for, into the
//! voice cache `dub` and `build` read from. And the video as it will play:
//! the manifest `dub` publishes, which the page plays as an outside
//! renderer would, so it cannot show timing the video will not have.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use teleprompt_cache::VoiceCache;
use teleprompt_compile::publish::{self, LineAudio, Published};
use teleprompt_compile::NarrationDetail;
use teleprompt_manifest::NarrationManifest;
use teleprompt_voice::takes::Takes;
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
    /// Why the script, as its file now reads, does not compile; `None` once
    /// it does again.
    error: Mutex<Option<Vec<String>>>,
    /// The manifest last made, and the audio it names.
    published: Mutex<Option<Published>>,
    runtime: OnceLock<Result<tokio::runtime::Runtime, String>>,
}

#[derive(Clone)]
struct Voiced {
    /// The narrator's backend.
    backend: Arc<dyn VoiceBackend>,
    /// Every backend a line is spoken by, the narrator's included.
    voices: crate::voice::Voices,
    lines: Vec<NarrationDetail>,
    length_ms: u64,
    /// Each line and shot in time, as [`timeline_json`] says it.
    timeline: serde_json::Value,
}

impl Voicing {
    pub fn new(project: &Project, script: &std::path::Path, locale: &str) -> Self {
        Self {
            project: project.clone(),
            script: script.to_path_buf(),
            locale: locale.to_string(),
            voiced: Mutex::new(None),
            error: Mutex::new(None),
            published: Mutex::new(None),
            runtime: OnceLock::new(),
        }
    }

    fn cache(&self) -> VoiceCache {
        VoiceCache::new(self.project.caches().root)
    }

    /// The script compiled as it now reads, or as it last compiled.
    fn refresh(&self) -> Option<Voiced> {
        let mut voiced = self.voiced.lock().unwrap_or_else(PoisonError::into_inner);
        let backends = self.project.backends();
        let compiled = self
            .project
            .compile_with(&backends, &self.script, &self.locale);
        match compiled {
            Ok((compiled, backend)) => {
                let voices = backends
                    .voices(&backend, &compiled.narration)
                    .unwrap_or_default();
                *voiced = Some(Voiced {
                    backend,
                    voices,
                    length_ms: compiled.timeline.duration_ms.ms(),
                    timeline: timeline_json(&compiled.timeline),
                    lines: compiled.narration,
                });
                *lock(&self.error) = None;
            }
            Err(errors) => *lock(&self.error) = Some(errors),
        }
        voiced.clone()
    }

    fn runtime(&self) -> Result<&tokio::runtime::Runtime, String> {
        self.runtime
            .get_or_init(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                    .map_err(|e| format!("cannot start the voice's runtime: {e}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The manifest `dub` would publish for the script as its file now
    /// reads, every line's audio made first; or, if it no longer compiles,
    /// the one last made, and why only if there is none.
    pub fn manifest(&self) -> Result<NarrationManifest, Vec<String>> {
        let backends = self.project.backends();
        let compile = || {
            self.project
                .compile_with(&backends, &self.script, &self.locale)
        };
        let last = || lock(&self.published).as_ref().map(|p| p.manifest.clone());
        let (compiled, backend) = match compile() {
            Ok(compiled) => compiled,
            Err(errors) => return last().ok_or(errors),
        };
        let voices = backends
            .voices(&backend, &compiled.narration)
            .map_err(|e| vec![e])?;
        let synthesized = self
            .runtime()
            .map_err(|e| vec![e])?
            .block_on(voiced(&voices, &self.cache(), &compiled.narration))
            .map_err(|e| vec![e])?;
        let takes = Takes::load(&self.project.takes_dir()).map_err(|e| vec![e.to_string()])?;
        let audio =
            publish::with_takes(&compiled.narration, synthesized, &takes).map_err(|e| vec![e])?;
        // Compiled again, as `dub` does: the first pass read durations from
        // a cache not yet filled.
        let (compiled, _) = compile()?;
        let published = publish::publish(&compiled, audio).map_err(|e| vec![e])?;
        let manifest = published.manifest.clone();
        *lock(&self.published) = Some(published);
        Ok(manifest)
    }

    /// Who reads the script, how long it runs, and each line's audio by
    /// id, for the script the prompter serves.
    pub fn describe(&self) -> Option<Description> {
        let voiced = self.refresh()?;
        // The narrator's: the voice of a line no speaker says.
        let error = lock(&self.error).clone();
        let voice = voiced
            .lines
            .iter()
            .filter(|l| l.speaker.is_none())
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
            timeline: voiced.timeline,
            error,
        })
    }

    /// Line `id`'s audio as a WAV, as [`Self::voice`] makes it; with `fit`,
    /// as the last manifest publishes it, in the video's format and a
    /// `fit-line` line at its tempo, as `dub` writes it.
    pub fn audio(&self, id: &str, fresh: bool, fit: bool) -> Result<Option<Vec<u8>>, String> {
        let published = (fit && !fresh)
            .then(|| {
                lock(&self.published).as_ref().and_then(|p| {
                    p.lines
                        .iter()
                        .find(|(line, _)| line == id)
                        .map(|(_, wav)| wav.clone())
                })
            })
            .flatten();
        match published {
            Some(wav) => Ok(Some(wav)),
            None => self.voice(id, fresh),
        }
    }

    /// Line `id`'s audio as a WAV: its take, or its voice's, made now if
    /// the cache lacks it or `fresh` asks for it anew. `None` for a line
    /// the script does not have.
    fn voice(&self, id: &str, fresh: bool) -> Result<Option<Vec<u8>>, String> {
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
        let runtime = self.runtime()?;
        let backend = voiced
            .voices
            .get(&line.backend)
            .ok_or_else(|| format!("line `{id}`: no voice backend `{}`", line.backend))?;
        let stored =
            runtime.block_on(crate::cmd::dub::synthesize_and_store(backend, &cache, line))?;
        Ok(Some(stored.wav))
    }
}

/// [`Voicing::describe`]'s answer.
pub struct Description {
    pub name: String,
    pub length_ms: u64,
    pub lines: Vec<(String, serde_json::Value)>,
    pub timeline: serde_json::Value,
    /// Why the script as its file now reads does not compile, if it does not.
    pub error: Option<Vec<String>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The audio of every line without a take, in document order: from the
/// voice cache, or synthesized into it now, one after another.
async fn voiced(
    voices: &crate::voice::Voices,
    cache: &VoiceCache,
    narration: &[NarrationDetail],
) -> Result<Vec<LineAudio>, String> {
    let mut audio = Vec::new();
    for detail in narration.iter().filter(|d| d.take.is_none()) {
        let failed = |e: &dyn std::fmt::Display| format!("line `{}`: {e}", detail.line_id);
        let cached = match cache
            .lookup(&detail.cache_key)
            .map_err(|e| failed(&e))?
            .hit()
        {
            Some(hit) => hit,
            None => {
                let backend = voices
                    .get(&detail.backend)
                    .ok_or_else(|| failed(&format!("no voice backend `{}`", detail.backend)))?;
                crate::cmd::dub::synthesize_and_store(backend, cache, detail).await?
            }
        };
        audio.push(LineAudio {
            line_id: detail.line_id.clone(),
            wav: cached.wav,
            duration_ms: cached.duration_ms,
            sample_rate: cached.sample_rate,
            channels: cached.channels,
        });
    }
    Ok(audio)
}

/// The scheduler's plan as a prompter draws it on the glass: each line
/// from its first sound to its last, and each shot with the line it runs
/// with or after, and whether it states its own length, so can stretch.
fn timeline_json(plan: &teleprompt_schedule::Timeline) -> serde_json::Value {
    let (mut lines, mut shots) = (Vec::new(), Vec::new());
    for entry in &plan.entries {
        let line = entry.narration.as_ref().map(|n| n.line.to_string());
        if let Some(n) = &entry.narration {
            lines.push(serde_json::json!({
                "id": n.line, "start_ms": n.start_ms.ms(),
                "end_ms": (n.start_ms + n.duration_ms).ms(),
            }));
        }
        if let Some(a) = &entry.action {
            shots.push(serde_json::json!({
                "shot": a.shot, "block": a.shot.block(), "scene": a.scene, "line": line,
                "start_ms": a.start_ms.ms(), "end_ms": (a.start_ms + a.duration_ms).ms(),
                "timed": a.duration_source != teleprompt_core::DurationSource::Unknown,
            }));
        }
    }
    serde_json::json!({ "duration_ms": plan.duration_ms.ms(), "lines": lines, "shots": shots })
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
            "speaker": line.speaker,
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
        "speaker": line.speaker,
    })
}
