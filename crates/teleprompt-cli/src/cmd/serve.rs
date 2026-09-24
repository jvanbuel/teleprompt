//! `teleprompt serve`: watch the script, recompile on save, synthesize only
//! what changed, and point a browser preview at the item that moved.
//!
//! The preview is a manifest consumer: it reads the narration manifest, as
//! `docs/integrations/remotion.md` describes it, plus shot sources. Nothing
//! here has privileged access to the schedule, so the preview cannot show
//! timing that a real consumer would not.
//!
//! The HTTP server is hand-rolled over `TcpListener`: it serves one browser
//! on loopback, five routes, and a framework would be the workspace's
//! largest dependency.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use teleprompt_cache::VoiceCache;
use teleprompt_compile::manifest::{self, AudioInfo, NarrationManifest};
use teleprompt_core::Hash;
use teleprompt_voice::VoiceBackend;

use crate::cmd::check::{backends_of, cache_root, compile_script_with};
use crate::project::Project;

/// How often the watcher reads the script. Polling rather than an OS watcher
/// costs one read per tick, and a quarter second is instant enough.
const POLL: Duration = Duration::from_millis(250);

const PAGE: &str = include_str!("serve.html");

#[derive(Debug)]
pub enum ServeError {
    /// The script did not compile on the first run. Later failures are shown
    /// in the preview instead.
    Validation(Vec<String>),
    Runtime(String),
}

/// What the preview reads. One generation per successful compile.
struct Preview {
    generation: u64,
    manifest: NarrationManifest,
    /// Shot id to adapter-native source, for the scenes the preview draws.
    shots: BTreeMap<String, String>,
    /// Line id to the cache key its audio lives under, as the compile
    /// computed it; the cache is addressed by content, not by line id.
    audio_keys: BTreeMap<String, String>,
    /// Ids of the items and lines whose timing or content moved in the
    /// compile that produced this generation. Empty on the first one.
    changed: Vec<String>,
    /// Set when the last save failed to compile. The last manifest that
    /// compiled stays served beside it.
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
    shots: BTreeMap<String, String>,
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

    // Compiled again, as `dub` does: the first pass read durations from a
    // cache `warm` had not yet filled, so they were estimates, and every
    // line would "move" on the next save.
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
            .map(|d| (d.line_id.clone(), d.cache_key.to_string()))
            .collect(),
        shots: compiled
            .shots
            .into_iter()
            .map(|s| (s.id, s.source))
            .collect(),
    })
}

/// Fills cache misses, and reports the audio shape the manifest publishes.
///
/// Sequential, unlike `dub`: after the first run the usual miss is the one
/// paragraph just edited, and `dub` is the command for a cold cache.
async fn warm(
    backend: &Arc<dyn VoiceBackend>,
    cache: &VoiceCache,
    compiled: &teleprompt_compile::CompileOutput,
) -> Result<AudioInfo, ServeError> {
    let mut shape: Option<(u32, u16)> = None;
    for detail in &compiled.narration {
        let hit = cache
            .lookup_meta(&detail.cache_key)
            .map_err(|e| ServeError::Runtime(format!("line `{}`: {e}", detail.line_id)))?
            .hit();
        let (rate, channels) = match hit {
            Some(meta) => (meta.sample_rate, meta.channels),
            None => {
                let s = backend
                    .synthesize(&detail.synth_request)
                    .await
                    .map_err(|e| ServeError::Runtime(format!("line `{}`: {e}", detail.line_id)))?;
                cache
                    .store(&detail.cache_key, &s.pcm, s.word_timings.as_deref())
                    .map_err(|e| ServeError::Runtime(format!("line `{}`: {e}", detail.line_id)))?;
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
/// Coarser than `diff` on purpose: the preview needs only somewhere to jump.
fn moved(before: &NarrationManifest, after: &NarrationManifest) -> Vec<String> {
    let mut out = Vec::new();
    for seg in &after.lines {
        match before.lines.iter().find(|s| s.id == seg.id) {
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
    for shot in &after.shots {
        match before.shots.iter().find(|b| b.shot == shot.shot) {
            None => out.push(shot.shot.clone()),
            Some(was) => {
                if was.capture_key != shot.capture_key
                    || was.start_ms != shot.start_ms
                    || was.duration_ms != shot.duration_ms
                {
                    out.push(shot.shot.clone());
                }
            }
        }
    }
    out
}

/// A request line's path: query stripped, percent-decoding applied. `None`
/// for anything that is not a well-formed HTTP GET this server serves.
///
/// A shot id contains a `#` (`welcome-a#0`), which arrives as `%23`.
fn route(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let path = parts.next()?;
    let path = path.split(['?', '#']).next().unwrap_or(path);
    decode_percent(path)
}

/// Percent-decoding. A malformed escape or invalid UTF-8 is `None`.
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

/// The line id in `/audio/<id>.wav`, rejecting anything that could escape
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

fn shot_id(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/shots/")?;
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

        p if shot_id(p).is_some() => {
            let id = shot_id(p).expect("just checked");
            let preview = state.lock().expect("preview lock");
            match preview.shots.get(id) {
                Some(source) => respond(
                    stream,
                    "200 OK",
                    "text/plain; charset=utf-8",
                    source.as_bytes(),
                ),
                None => respond(stream, "404 Not Found", "text/plain", b"no such shot"),
            }
        }

        p if audio_id(p).is_some() => {
            let id = audio_id(p).expect("just checked");
            // The key the compile computed; recomputing it here would be a
            // second answer to the same question.
            let key = {
                let preview = state.lock().expect("preview lock");
                preview.audio_keys.get(id).cloned()
            };
            let Some(key) = key else {
                return respond(stream, "404 Not Found", "text/plain", b"no such line");
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

/// [`run_serve`] against a listener the caller already bound, so a test can
/// take port 0 and drive the real server.
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
        shots: built.shots,
        audio_keys: built.audio_keys,
        changed: Vec::new(),
        error: None,
    }));

    eprintln!("  watching {}", script.display());

    let script_name = script
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // The listener gets its own OS thread. `incoming()` blocks, and the
    // runtime is current-thread: an accept loop there would park the
    // executor, and the watcher below would never be polled again — a
    // server that works and never notices a save.
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
    let mut seen = fingerprint(&script);
    loop {
        tokio::time::sleep(POLL).await;
        let now = fingerprint(&script);
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
                p.shots = built.shots;
                p.audio_keys = built.audio_keys;
                p.changed = built.changed;
                p.error = None;
                eprintln!("  recompiled — {} item(s) moved", p.changed.len());
            }
            Err(ServeError::Validation(errors)) => {
                // Keep the last manifest that compiled: the author wants the
                // error beside the video they had, not a blank screen.
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

/// What the watcher compares between ticks: the script's contents, not its
/// mtime. An edit within the filesystem's timestamp resolution leaves the
/// mtime unchanged, and that save is then never seen at all.
fn fingerprint(path: &Path) -> Option<Hash> {
    std::fs::read(path).ok().map(|bytes| Hash::of(&bytes))
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
    fn a_shot_id_survives_the_encoding_a_client_must_apply() {
        // `welcome-a#0` has to arrive as `%23`, or the fragment split above
        // would eat everything after the `-a`.
        let path = route("GET /shots/welcome-a%230 HTTP/1.1").expect("decodes");
        assert_eq!(shot_id(&path), Some("welcome-a#0"));
        assert_eq!(route("GET /shots/bad%2 HTTP/1.1"), None);
        assert_eq!(route("GET /shots/bad%zz HTTP/1.1"), None);
    }

    #[test]
    fn an_audio_path_cannot_escape_the_cache() {
        assert_eq!(audio_id("/audio/welcome.wav"), Some("welcome"));
        assert_eq!(audio_id("/audio/the-loop.wav"), Some("the-loop"));
        // A line id is `[a-z0-9-_]` by construction, so anything that
        // could walk out of the cache directory is not one.
        assert_eq!(audio_id("/audio/../../etc/passwd.wav"), None);
        assert_eq!(audio_id("/audio/.wav"), None);
        assert_eq!(audio_id("/audio/welcome.mp3"), None);
    }

    #[test]
    fn a_span_path_admits_the_hash_a_shot_id_carries() {
        assert_eq!(shot_id("/shots/welcome-a#0"), Some("welcome-a#0"));
        assert_eq!(shot_id("/shots/../secrets"), None);
        assert_eq!(shot_id("/shots/"), None);
    }
}
