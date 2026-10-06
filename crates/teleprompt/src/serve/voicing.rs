//! How each line of a prompted script sounds when its voice reads it: the
//! prompter's view of what `dub` would make. A line with a current take is
//! read from it; any other is synthesized when first asked for, into the
//! voice cache `dub` and `build` read from. And the video as it will play:
//! the manifest `dub` publishes, which the page plays as an outside
//! renderer would, so it cannot show timing the video will not have.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::UNIX_EPOCH;

use crate::dub::publish::Published;
use teleprompt_compile::NarrationDetail;
use teleprompt_manifest::NarrationManifest;
use teleprompt_voice::cache::VoiceCache;
use teleprompt_voice::VoiceBackend;

use crate::dub;
use crate::project::{fingerprint, translation_path, Compiled, Script};
use crate::Failure;
use teleprompt_core::Hash;

/// A script's voice, compiled afresh when what it is compiled from has
/// changed: the script, its translation, its takes, or which of its lines
/// the voice cache holds.
pub struct Voicing {
    script: Script,
    /// The last compile: each line's request and take, and who reads.
    voiced: Mutex<Option<Voiced>>,
    /// What the last compile was made from, as [`Voicing::inputs`] says it.
    inputs: Mutex<Option<Hash>>,
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
    pub fn new(script: Script) -> Self {
        Self {
            script,
            voiced: Mutex::new(None),
            inputs: Mutex::new(None),
            error: Mutex::new(None),
            published: Mutex::new(None),
            runtime: OnceLock::new(),
        }
    }

    fn cache(&self) -> VoiceCache {
        VoiceCache::new(self.script.project().caches().root)
    }

    /// What a compile reads beyond the project's config, which is read
    /// once: the script, its translation, the takes, and which of the
    /// lines last compiled the voice cache now holds.
    fn inputs(&self, last: Option<&Voiced>) -> Hash {
        let mut fields = vec![
            fingerprint(self.script.path()),
            fingerprint(&translation_path(self.script.path(), self.script.locale())),
        ]
        .into_iter()
        .map(|h| h.map_or_else(String::new, |h| h.to_string()))
        .collect::<Vec<_>>();
        let takes = std::fs::read_dir(self.script.project().takes_dir())
            .into_iter()
            .flatten();
        let mut takes: Vec<String> = takes
            .flatten()
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
                let name = e.file_name().to_string_lossy().into_owned();
                Some(format!("{name}:{}:{}", meta.len(), modified.as_nanos()))
            })
            .collect();
        takes.sort();
        fields.extend(takes);
        let cache = self.cache();
        let held = last.map_or(&[][..], |v| &v.lines[..]);
        fields.push(
            held.iter()
                .map(|l| if cache.has(&l.cache_key) { '1' } else { '0' })
                .collect(),
        );
        Hash::of_fields(&fields.iter().map(String::as_str).collect::<Vec<_>>())
    }

    /// The script compiled as it now reads, or as it last compiled; only
    /// compiled again when what it is compiled from has changed.
    fn refresh(&self) -> Option<Voiced> {
        let mut voiced = self.voiced.lock().unwrap_or_else(PoisonError::into_inner);
        let inputs = self.inputs(voiced.as_ref());
        let mut seen = lock(&self.inputs);
        if seen.as_ref() == Some(&inputs) {
            return voiced.clone();
        }
        *seen = Some(inputs);
        match self.script.compile() {
            Ok(Compiled {
                output: compiled,
                backend,
                ..
            }) => {
                let voices = self
                    .script
                    .backends()
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
        let voiced = self
            .runtime()
            .map_err(|e| vec![e])?
            .block_on(dub::voice(&self.script));
        let published = match voiced {
            Ok(voiced) => voiced.published,
            Err(e) => {
                let last = lock(&self.published).as_ref().map(|p| p.manifest.clone());
                return match (e, last) {
                    // A script that no longer compiles keeps playing as it last did.
                    (Failure::Validation(_), Some(last)) => Ok(last),
                    (Failure::Validation(errors), None) => Err(errors),
                    (Failure::Runtime(e), _) => Err(vec![e]),
                };
            }
        };
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
            let take = self.script.project().takes_dir().join(format!("{id}.wav"));
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
        let stored = runtime.block_on(dub::synthesize_and_store(backend, &cache, line))?;
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
        let words = crate::serve::prompter::word_starts(&line.text, take.duration_ms);
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
                _ => crate::serve::prompter::word_starts(&line.text, meta.duration_ms),
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
