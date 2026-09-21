//! `teleprompt serve` — the loop you can watch.
//!
//! `plan` and `diff` answer in milliseconds and tell you numbers. Judging an
//! edit means hearing it, which cost a re-dub and a render — minutes, for a
//! change you wanted to evaluate in seconds. `serve` closes that: it watches
//! the script, recompiles on save, synthesizes only what changed, and hands a
//! preview the beat that moved.
//!
//! **The preview is a manifest consumer.** It reads exactly what
//! `docs/integrations/remotion.md` tells an outside integrator to read —
//! `narration.json` with its `beats` — plus the span sources it needs to draw
//! a scene. Nothing here has privileged access to the schedule, which is what
//! keeps "looked fine in preview" from becoming a category of bug.
//!
//! The HTTP server is hand-rolled over `TcpListener`. It serves one machine's
//! own browser on loopback: a framework would be the largest dependency in the
//! workspace, for four routes and no concurrency to speak of.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use teleprompt_cache::VoiceCache;
use teleprompt_compile::manifest::{self, AudioInfo, NarrationManifest};
use teleprompt_voice::VoiceBackend;

use crate::cmd::check::{backends_of, cache_root, compile_script_with};
use crate::project::Project;

/// How often the watcher looks at the script's modification time.
///
/// Polling rather than an OS watcher: the whole dependency is one `metadata`
/// call per file per tick, and a preview that reacts within a quarter second
/// of a save is indistinguishable from one that reacts instantly.
const POLL: Duration = Duration::from_millis(250);

const PAGE: &str = include_str!("serve.html");

#[derive(Debug)]
pub enum ServeError {
    /// The script did not compile on the first run. Later failures do not
    /// stop the server — they are shown in the preview and cleared by the
    /// next save that fixes them.
    Validation(Vec<String>),
    Runtime(String),
}

/// What the preview reads. One generation per successful compile.
struct Preview {
    generation: u64,
    manifest: NarrationManifest,
    /// Span id to adapter-native source, for the scenes the preview draws.
    spans: BTreeMap<String, String>,
    /// Segment id to the cache key its audio lives under. The cache is
    /// addressed by content, so this is the index from the name a consumer
    /// asks for to the file that answers it.
    audio_keys: BTreeMap<String, String>,
    /// Ids of the beats and segments whose timing or content moved in the
    /// compile that produced this generation. Empty on the first one.
    changed: Vec<String>,
    /// Set when the last save failed to compile. The previous generation's
    /// manifest stays served, so the preview keeps playing what last worked
    /// while showing what is wrong.
    error: Option<Vec<String>>,
}

#[derive(Serialize)]
struct State<'a> {
    generation: u64,
    script: &'a str,
    duration_ms: u64,
    changed: &'a [String],
    error: Option<&'a Vec<String>>,
}

/// What one compile produces for the preview.
struct Built {
    manifest: NarrationManifest,
    spans: BTreeMap<String, String>,
    audio_keys: BTreeMap<String, String>,
    changed: Vec<String>,
}

/// Compile, synthesize anything the cache is missing, and publish a preview.
async fn rebuild(
    project: &Project,
    script: &Path,
    locale: &str,
    previous: Option<&NarrationManifest>,
) -> Result<Built, ServeError> {
    let backends = backends_of(project);
    let (compiled, backend) =
        compile_script_with(&backends, project, script, locale).map_err(ServeError::Validation)?;

    let cache = VoiceCache::new(cache_root(project));
    let audio = warm(&backend, &cache, &compiled).await?;

    // Compiled a second time, because the first one read a cache that did
    // not yet hold what `warm` has just put in it: its durations are the
    // estimator's guesses, and publishing those would make the first
    // preview play at a pace the second one corrects. Every segment would
    // then "move" on the first save, and the preview would have nowhere
    // meaningful to jump to. `dub` recompiles after rendering for the same
    // reason. The second pass is offline and sub-second — that is what the
    // inner loop is built to be.
    let (compiled, _) =
        compile_script_with(&backends, project, script, locale).map_err(ServeError::Validation)?;

    let manifest = manifest::build(
        &compiled.timeline,
        &compiled.chapters,
        &compiled.narration,
        audio,
    );
    let changed = previous.map(|p| moved(p, &manifest)).unwrap_or_default();

    Ok(Built {
        changed,
        manifest,
        audio_keys: compiled
            .narration
            .iter()
            .map(|d| (d.segment_id.clone(), d.cache_key.to_string()))
            .collect(),
        spans: compiled
            .spans
            .into_iter()
            .map(|s| (s.id, s.source))
            .collect(),
    })
}

/// Fills cache misses, and reports the audio shape the manifest publishes.
///
/// Sequential on purpose, which is the one place this differs from `dub`:
/// after the first run the common case is exactly one miss — the paragraph
/// just edited — and a fan-out with a semaphore, completion-order progress
/// and per-key grouping buys nothing for a single request. A cold start pays
/// for that simplicity once; `dub` is the command for a cold start.
async fn warm(
    backend: &Arc<dyn VoiceBackend>,
    cache: &VoiceCache,
    compiled: &teleprompt_compile::CompileOutput,
) -> Result<AudioInfo, ServeError> {
    let mut shape: Option<(u32, u16)> = None;
    for detail in &compiled.narration {
        let hit = cache
            .lookup_meta(&detail.cache_key)
            .map_err(|e| ServeError::Runtime(format!("segment `{}`: {e}", detail.segment_id)))?
            .hit();
        let (rate, channels) = match hit {
            Some(meta) => (meta.sample_rate, meta.channels),
            None => {
                let s = backend
                    .synthesize(&detail.synth_request)
                    .await
                    .map_err(|e| {
                        ServeError::Runtime(format!("segment `{}`: {e}", detail.segment_id))
                    })?;
                cache
                    .store(&detail.cache_key, &s.pcm, s.word_timings.as_deref())
                    .map_err(|e| {
                        ServeError::Runtime(format!("segment `{}`: {e}", detail.segment_id))
                    })?;
                (s.pcm.sample_rate, s.pcm.channels)
            }
        };
        shape.get_or_insert((rate, channels));
    }

    let (sample_rate, channels) = shape.unwrap_or((24_000, 1));
    Ok(AudioInfo {
        format: "wav".to_string(),
        sample_rate,
        channels,
    })
}

/// What moved between two manifests, as ids the preview can seek to.
///
/// Deliberately coarser than `diff`: the preview only needs somewhere to
/// jump, so a segment whose text or timing changed and a beat whose schedule
/// moved are both simply "changed". `diff` remains the surface for reading
/// *what* changed.
fn moved(before: &NarrationManifest, after: &NarrationManifest) -> Vec<String> {
    let mut out = Vec::new();
    for seg in &after.segments {
        match before.segments.iter().find(|s| s.id == seg.id) {
            None => out.push(seg.id.clone()),
            Some(was) => {
                if was.source_hash != seg.source_hash
                    || was.start_ms != seg.start_ms
                    || was.duration_ms != seg.duration_ms
                {
                    out.push(seg.id.clone());
                }
            }
        }
    }
    for beat in &after.beats {
        match before.beats.iter().find(|b| b.span == beat.span) {
            None => out.push(beat.span.clone()),
            Some(was) => {
                if was.span_hash != beat.span_hash
                    || was.start_ms != beat.start_ms
                    || was.duration_ms != beat.duration_ms
                {
                    out.push(beat.span.clone());
                }
            }
        }
    }
    out
}

/// A request line's path: query stripped, percent-decoding applied. `None`
/// for anything that is not a well-formed HTTP GET this server serves.
///
/// The decode is not optional politeness: a span id contains a `#`
/// (`welcome-a#0`), which a client must percent-encode, so without this
/// every span request arrives as `%23` and matches nothing.
fn route(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let path = parts.next()?;
    let path = path.split(['?', '#']).next().unwrap_or(path);
    decode_percent(path)
}

/// Percent-decoding, rejecting anything that is not valid UTF-8 or not a
/// well-formed escape — a malformed path is a bad request, not something to
/// guess at.
fn decode_percent(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The segment id in `/audio/<id>.wav`, rejecting anything that could escape
/// the cache: this server answers a browser, and a path is user input even
/// when the user is its own author.
fn audio_id(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/audio/")?.strip_suffix(".wav")?;
    let ok = !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then_some(rest)
}

fn span_id(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/spans/")?;
    let ok = !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '#'));
    ok.then_some(rest)
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn handle(
    stream: &mut TcpStream,
    state: &Mutex<Preview>,
    script_name: &str,
    cache_dir: &Path,
) -> std::io::Result<()> {
    let mut line = String::new();
    BufReader::new(&*stream).read_line(&mut line)?;

    let Some(path) = route(&line) else {
        return respond(
            stream,
            "400 Bad Request",
            "text/plain",
            b"malformed request",
        );
    };

    match path.as_str() {
        "/" => respond(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
        ),

        "/state.json" => {
            let p = state.lock().expect("preview lock");
            let body = serde_json::to_vec(&State {
                generation: p.generation,
                script: script_name,
                duration_ms: p.manifest.duration_ms,
                changed: &p.changed,
                error: p.error.as_ref(),
            })
            .expect("state serializes");
            respond(stream, "200 OK", "application/json", &body)
        }

        "/manifest.json" => {
            let p = state.lock().expect("preview lock");
            let body = serde_json::to_vec(&p.manifest).expect("manifest serializes");
            respond(stream, "200 OK", "application/json", &body)
        }

        p if span_id(p).is_some() => {
            let id = span_id(p).expect("just checked");
            let preview = state.lock().expect("preview lock");
            match preview.spans.get(id) {
                Some(source) => respond(
                    stream,
                    "200 OK",
                    "text/plain; charset=utf-8",
                    source.as_bytes(),
                ),
                None => respond(stream, "404 Not Found", "text/plain", b"no such span"),
            }
        }

        p if audio_id(p).is_some() => {
            let id = audio_id(p).expect("just checked");
            // The cache is addressed by content, not by segment id, so the
            // key index built during the compile is what turns the name a
            // consumer asks for into the file that answers it. Reconstructing
            // the key here would be a second answer to a question the compile
            // already answered.
            let key = {
                let preview = state.lock().expect("preview lock");
                preview.audio_keys.get(id).cloned()
            };
            let Some(key) = key else {
                return respond(stream, "404 Not Found", "text/plain", b"no such segment");
            };
            match std::fs::read(cache_dir.join("voice").join(format!("{key}.wav"))) {
                Ok(bytes) => respond(stream, "200 OK", "audio/wav", &bytes),
                Err(_) => respond(stream, "404 Not Found", "text/plain", b"no audio yet"),
            }
        }

        _ => respond(stream, "404 Not Found", "text/plain", b"not found"),
    }
}

/// Runs until interrupted. Returns only on a failure that makes serving
/// pointless — the first compile, or the listener itself.
pub async fn run_serve(
    project: &Project,
    script: &Path,
    locale: &str,
    port: u16,
) -> Result<(), ServeError> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = TcpListener::bind(addr)
        .map_err(|e| ServeError::Runtime(format!("cannot listen on {addr}: {e}")))?;
    let bound = listener
        .local_addr()
        .map_err(|e| ServeError::Runtime(e.to_string()))?;
    eprintln!("teleprompt serve — http://{bound}");
    serve_on(listener, project, script, locale).await
}

/// [`run_serve`] against a listener the caller already bound.
///
/// The seam exists so a test can take port 0, learn which port the OS gave
/// it, and drive the real server — rather than testing a reimplementation of
/// it, or racing a hardcoded port against whatever else is running.
pub async fn serve_on(
    listener: TcpListener,
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<(), ServeError> {
    let built = rebuild(project, script, locale, None).await?;
    let cache_dir = cache_root(project);

    let state = Arc::new(Mutex::new(Preview {
        generation: 1,
        manifest: built.manifest,
        spans: built.spans,
        audio_keys: built.audio_keys,
        changed: Vec::new(),
        error: None,
    }));

    eprintln!("  watching {}", script.display());

    let script_name = script
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // The listener gets its own OS thread and the watcher keeps the runtime.
    //
    // `TcpListener::incoming()` blocks, and the CLI's runtime is
    // current-thread: an accept loop on the async side parks the executor,
    // and the watcher — a `tokio::spawn`ed task — is simply never polled
    // again. It looks like a server that works and never notices a save,
    // which is the one thing this command exists to do. Nothing here is
    // async, so a plain thread is the honest home for it.
    {
        let state = state.clone();
        let script_name = script_name.clone();
        let cache_dir = cache_dir.clone();
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let mut stream = match incoming {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("warning: dropped connection: {e}");
                        continue;
                    }
                };
                if let Err(e) = handle(&mut stream, &state, &script_name, &cache_dir) {
                    eprintln!("warning: {e}");
                }
            }
        });
    }

    // Runs until interrupted.
    watch(
        state,
        project.clone(),
        script.to_path_buf(),
        locale.to_string(),
    )
    .await;
    Ok(())
}

async fn watch(state: Arc<Mutex<Preview>>, project: Project, script: PathBuf, locale: String) {
    let mut seen = modified(&script);
    loop {
        tokio::time::sleep(POLL).await;
        let now = modified(&script);
        if now == seen {
            continue;
        }
        seen = now;

        let previous = {
            let p = state.lock().expect("preview lock");
            p.manifest.clone()
        };
        match rebuild(&project, &script, &locale, Some(&previous)).await {
            Ok(built) => {
                let mut p = state.lock().expect("preview lock");
                p.generation += 1;
                p.manifest = built.manifest;
                p.spans = built.spans;
                p.audio_keys = built.audio_keys;
                p.changed = built.changed;
                p.error = None;
                eprintln!("  recompiled — {} beat(s) moved", p.changed.len());
            }
            Err(ServeError::Validation(errors)) => {
                // The preview keeps playing the last thing that compiled.
                // A blank screen would answer a question nobody asked:
                // what the author wants to see is the error and the video
                // they had a moment ago.
                let mut p = state.lock().expect("preview lock");
                for e in &errors {
                    eprintln!("{e}");
                }
                p.error = Some(errors);
                p.generation += 1;
            }
            Err(ServeError::Runtime(e)) => {
                let mut p = state.lock().expect("preview lock");
                eprintln!("error: {e}");
                p.error = Some(vec![e]);
                p.generation += 1;
            }
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_strip_the_query_and_reject_anything_but_get() {
        assert_eq!(
            route("GET /state.json?since=3 HTTP/1.1").as_deref(),
            Some("/state.json")
        );
        assert_eq!(route("GET / HTTP/1.1").as_deref(), Some("/"));
        assert_eq!(route("POST /manifest.json HTTP/1.1"), None);
        assert_eq!(route("garbage"), None);
    }

    #[test]
    fn a_span_id_survives_the_encoding_a_client_must_apply() {
        // `welcome-a#0` has to arrive as `%23`, or the fragment split above
        // would eat everything after the `-a`.
        let path = route("GET /spans/welcome-a%230 HTTP/1.1").expect("decodes");
        assert_eq!(span_id(&path), Some("welcome-a#0"));
        assert_eq!(route("GET /spans/bad%2 HTTP/1.1"), None);
        assert_eq!(route("GET /spans/bad%zz HTTP/1.1"), None);
    }

    #[test]
    fn an_audio_path_cannot_escape_the_cache() {
        assert_eq!(audio_id("/audio/welcome.wav"), Some("welcome"));
        assert_eq!(audio_id("/audio/the-loop.wav"), Some("the-loop"));
        // A segment id is `[a-z0-9-_]` by construction, so anything that
        // could walk out of the cache directory is not one.
        assert_eq!(audio_id("/audio/../../etc/passwd.wav"), None);
        assert_eq!(audio_id("/audio/.wav"), None);
        assert_eq!(audio_id("/audio/welcome.mp3"), None);
    }

    #[test]
    fn a_span_path_admits_the_hash_a_span_id_carries() {
        assert_eq!(span_id("/spans/welcome-a#0"), Some("welcome-a#0"));
        assert_eq!(span_id("/spans/../secrets"), None);
        assert_eq!(span_id("/spans/"), None);
    }
}
