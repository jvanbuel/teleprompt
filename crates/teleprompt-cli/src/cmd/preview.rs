//! `teleprompt prompt --preview`: watch the script, recompile on save, synthesize only
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

use crate::project::fingerprint;
use serde::Serialize;
use teleprompt_cache::VoiceCache;
use teleprompt_compile::manifest;
use teleprompt_core::{Hash, LineId, ShotId};
use teleprompt_manifest::{AudioInfo, NarrationManifest};
use teleprompt_voice::VoiceBackend;

use crate::cmd::check::{backends_of, compile_script_with};
use crate::project::Project;

/// How often the watcher reads the script. Polling rather than an OS watcher
/// costs one read per tick, and a quarter second is instant enough.
const POLL: Duration = Duration::from_millis(250);

const PAGE: &str = include_str!("preview.html");

#[derive(Debug)]
pub enum PreviewError {
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
    shots: BTreeMap<ShotId, String>,
    /// Line id to the file its audio is in: its take, or the cache entry
    /// under the key the compile computed, since the cache is addressed by
    /// content, not by line id.
    audio_files: BTreeMap<LineId, PathBuf>,
    /// Ids of the items and lines whose timing or content moved in the
    /// compile that produced this generation. Empty on the first one.
    changed: Vec<String>,
    /// Set when the last save failed to compile. The last manifest that
    /// compiled stays served beside it.
    error: Option<Vec<String>>,
    /// A save is being compiled and its audio made.
    building: bool,
}

#[derive(Serialize)]
struct State<'a> {
    generation: u64,
    script: &'a str,
    duration_ms: u64,
    changed: &'a [String],
    error: Option<&'a Vec<String>>,
    building: bool,
}

/// What one compile produces for the preview.
struct Built {
    manifest: NarrationManifest,
    shots: BTreeMap<ShotId, String>,
    audio_files: BTreeMap<LineId, PathBuf>,
    changed: Vec<String>,
}

/// Compile, synthesize anything the cache is missing, and publish a preview.
async fn rebuild(
    project: &Project,
    script: &Path,
    locale: &str,
    previous: Option<&NarrationManifest>,
) -> Result<Built, PreviewError> {
    let backends = backends_of(project);
    let (compiled, backend) = compile_script_with(&backends, project, script, locale)
        .map_err(PreviewError::Validation)?;

    let cache = VoiceCache::new(project.caches().root);
    let audio = warm(&backend, &cache, &compiled).await?;

    // Compiled again, as `dub` does: the first pass read durations from a
    // cache `warm` had not yet filled, so they were estimates, and every
    // line would "move" on the next save.
    let (compiled, _) = compile_script_with(&backends, project, script, locale)
        .map_err(PreviewError::Validation)?;

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
        audio_files: compiled
            .narration
            .iter()
            .map(|d| {
                let file = match d.take {
                    Some(_) => project.takes_dir().join(format!("{}.wav", d.line_id)),
                    None => project
                        .caches()
                        .voice()
                        .join(format!("{}.wav", d.cache_key)),
                };
                (d.line_id.clone(), file)
            })
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
) -> Result<AudioInfo, PreviewError> {
    let mut shape: Option<(u32, u16)> = None;
    // A recorded line is played from its take.
    for detail in compiled.narration.iter().filter(|d| d.take.is_none()) {
        let hit = cache
            .lookup_meta(&detail.cache_key)
            .map_err(|e| PreviewError::Runtime(format!("line `{}`: {e}", detail.line_id)))?
            .hit();
        let (rate, channels) = match hit {
            Some(meta) => (meta.sample_rate, meta.channels),
            None => {
                let stored = crate::cmd::dub::synthesize_and_store(backend, cache, detail)
                    .await
                    .map_err(PreviewError::Runtime)?;
                (stored.sample_rate, stored.channels)
            }
        };
        shape.get_or_insert((rate, channels));
    }

    let (sample_rate, channels) = shape.unwrap_or((crate::cmd::dub::NO_AUDIO_SAMPLE_RATE, 1));
    Ok(AudioInfo {
        format: "wav".to_string(),
        sample_rate,
        channels,
    })
}

/// What moved between two manifests, as ids the preview can seek to.
/// Coarser than `plan --check` on purpose: the preview needs only somewhere to jump.
fn moved(before: &NarrationManifest, after: &NarrationManifest) -> Vec<String> {
    let mut out = Vec::new();
    for seg in &after.lines {
        match before.lines.iter().find(|s| s.id == seg.id) {
            None => out.push(seg.id.to_string()),
            Some(was) => {
                if was.source_hash != seg.source_hash
                    || was.start_ms != seg.start_ms
                    || was.duration_ms != seg.duration_ms
                {
                    out.push(seg.id.to_string());
                }
            }
        }
    }
    for shot in &after.shots {
        match before.shots.iter().find(|b| b.shot == shot.shot) {
            None => out.push(shot.shot.to_string()),
            Some(was) => {
                if was.capture_key != shot.capture_key
                    || was.start_ms != shot.start_ms
                    || was.duration_ms != shot.duration_ms
                {
                    out.push(shot.shot.to_string());
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

/// A `fit-line` line's audio at the tempo and length the manifest gives it,
/// as `dub` writes it; any other line's as it is.
fn at_tempo(bytes: Vec<u8>, tempo: Option<(u32, u64)>) -> Vec<u8> {
    tempo
        .and_then(|(tempo, ms)| teleprompt_voice::stretch::fit_wav(&bytes, tempo, ms).ok())
        .unwrap_or(bytes)
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

/// The request line, and the `Host` and `Origin` headers.
type Head = (String, Option<String>, Option<String>);

fn read_head(stream: &TcpStream) -> std::io::Result<Head> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let (mut host, mut origin) = (None, None);
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = Some(value.trim().to_string());
            match name.trim().to_ascii_lowercase().as_str() {
                "host" => host = value,
                "origin" => origin = value,
                _ => {}
            }
        }
    }
    Ok((line, host, origin))
}

fn handle(
    stream: &mut TcpStream,
    state: &Mutex<Preview>,
    script_name: &str,
) -> std::io::Result<()> {
    let (line, host, origin) = read_head(stream)?;
    if let Some(why) = crate::loopback::refused(host.as_deref(), origin.as_deref()) {
        return respond(stream, "403 Forbidden", "text/plain", why.as_bytes());
    }

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

        // Each body is made under the lock and sent after it is let go: a
        // reader that stops reading must not stall the watcher.
        "/state.json" => {
            let body = {
                let p = state.lock().expect("preview lock");
                serde_json::to_vec(&State {
                    generation: p.generation,
                    script: script_name,
                    duration_ms: p.manifest.duration_ms.ms(),
                    changed: &p.changed,
                    error: p.error.as_ref(),
                    building: p.building,
                })
                .expect("state serializes")
            };
            respond(stream, "200 OK", "application/json", &body)
        }

        crate::cmd::prompt::FONT_PATH => {
            respond(stream, "200 OK", "font/woff2", crate::cmd::prompt::FONT)
        }

        "/manifest.json" => {
            let body = {
                let p = state.lock().expect("preview lock");
                serde_json::to_vec(&p.manifest).expect("manifest serializes")
            };
            respond(stream, "200 OK", "application/json", &body)
        }

        p if shot_id(p).is_some() => {
            let id = shot_id(p).expect("just checked");
            let source = {
                let preview = state.lock().expect("preview lock");
                preview.shots.get(id).cloned()
            };
            match source {
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
            // The file the compile resolved; recomputing it here would be a
            // second answer to the same question.
            let file = {
                let preview = state.lock().expect("preview lock");
                preview.audio_files.get(id).cloned()
            };
            let Some(file) = file else {
                return respond(stream, "404 Not Found", "text/plain", b"no such line");
            };
            let tempo = {
                let preview = state.lock().expect("preview lock");
                preview
                    .manifest
                    .lines
                    .iter()
                    .find(|l| l.id == id)
                    .and_then(|l| l.tempo_permille.map(|t| (t.permille(), l.duration_ms.ms())))
            };
            match std::fs::read(file) {
                Ok(bytes) => respond(stream, "200 OK", "audio/wav", &at_tempo(bytes, tempo)),
                Err(_) => respond(stream, "404 Not Found", "text/plain", b"no audio yet"),
            }
        }

        _ => respond(stream, "404 Not Found", "text/plain", b"not found"),
    }
}

/// Runs until interrupted. Returns only on a failure that makes serving
/// pointless — the first compile, or the listener itself.
pub async fn run_preview(
    project: &Project,
    script: &Path,
    locale: &str,
    port: u16,
) -> Result<(), PreviewError> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = TcpListener::bind(addr)
        .map_err(|e| PreviewError::Runtime(format!("cannot listen on {addr}: {e}")))?;
    let bound = listener
        .local_addr()
        .map_err(|e| PreviewError::Runtime(e.to_string()))?;
    eprintln!("previewing at http://{bound}/");
    preview_on(listener, project, script, locale).await
}

/// [`run_preview`] against a listener the caller already bound, so a test can
/// take port 0 and drive the real server.
pub async fn preview_on(
    listener: TcpListener,
    project: &Project,
    script: &Path,
    locale: &str,
) -> Result<(), PreviewError> {
    // Taken before the build reads the script, so a save during startup
    // differs from the baseline instead of becoming it.
    let compiled = fingerprint(script);
    let built = rebuild(project, script, locale, None).await?;

    let state = Arc::new(Mutex::new(Preview {
        generation: 1,
        manifest: built.manifest,
        shots: built.shots,
        audio_files: built.audio_files,
        changed: Vec::new(),
        error: None,
        building: false,
    }));

    eprintln!("  watching {}", script.display());

    let script_name = script
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // The listener gets its own OS thread. `incoming()` blocks, and the
    // runtime is current-thread: an accept loop there would park the
    // executor, and the watcher below would never be polled again — a
    // server that works and never notices a save. Each connection gets a
    // thread too, so a browser's idle one holds up no one.
    {
        let state = state.clone();
        let script_name: Arc<str> = script_name.clone().into();
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let mut stream = match incoming {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("warning: dropped connection: {e}");
                        continue;
                    }
                };
                let (state, script_name) = (state.clone(), script_name.clone());
                std::thread::spawn(move || {
                    let limit = Some(std::time::Duration::from_secs(10));
                    let answered = stream
                        .set_read_timeout(limit)
                        .and_then(|()| stream.set_write_timeout(limit))
                        .and_then(|()| handle(&mut stream, &state, &script_name));
                    // A browser opens connections it never uses; their
                    // timing out is not worth a warning.
                    let idle = |e: &std::io::Error| {
                        matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        )
                    };
                    match answered {
                        Err(e) if !idle(&e) => eprintln!("warning: {e}"),
                        _ => {}
                    }
                });
            }
        });
    }

    watch(
        state,
        project.clone(),
        script.to_path_buf(),
        locale.to_string(),
        compiled,
    )
    .await;
    Ok(())
}

async fn watch(
    state: Arc<Mutex<Preview>>,
    project: Project,
    script: PathBuf,
    locale: String,
    mut seen: Option<Hash>,
) {
    loop {
        tokio::time::sleep(POLL).await;
        let now = fingerprint(&script);
        if now == seen {
            continue;
        }
        seen = now;

        let previous = {
            let mut p = state.lock().expect("preview lock");
            p.building = true;
            p.manifest.clone()
        };
        let built = rebuild(&project, &script, &locale, Some(&previous)).await;
        state.lock().expect("preview lock").building = false;
        match built {
            Ok(built) => {
                let mut p = state.lock().expect("preview lock");
                p.generation += 1;
                p.manifest = built.manifest;
                p.shots = built.shots;
                p.audio_files = built.audio_files;
                p.changed = built.changed;
                p.error = None;
                eprintln!("  recompiled — {} item(s) moved", p.changed.len());
            }
            Err(PreviewError::Validation(errors)) => {
                // Keep the last manifest that compiled: the author wants the
                // error beside the video they had, not a blank screen.
                let mut p = state.lock().expect("preview lock");
                for e in &errors {
                    eprintln!("{e}");
                }
                p.error = Some(errors);
                p.generation += 1;
            }
            Err(PreviewError::Runtime(e)) => {
                let mut p = state.lock().expect("preview lock");
                eprintln!("error: {e}");
                p.error = Some(vec![e]);
                p.generation += 1;
            }
        }
    }
}

impl From<PreviewError> for crate::output::Outcome {
    fn from(e: PreviewError) -> Self {
        match e {
            PreviewError::Validation(errors) => Self::ValidationError(errors),
            PreviewError::Runtime(message) => Self::RuntimeFailure(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A save between the first build reading the script and the watcher
    /// starting is still seen, because the watcher's baseline is what the
    /// build compiled, not what the file holds when it starts.
    #[tokio::test]
    async fn an_edit_during_startup_is_not_lost() {
        let dir = teleprompt_testkit::test_dir("preview-startup");
        crate::cmd::new::scaffold(&dir).unwrap();
        let project = Project::discover(&dir).unwrap();
        let script = dir.join("scripts/demo.md");

        let compiled = fingerprint(&script);
        let built = rebuild(&project, &script, "en", None).await.unwrap();
        let state = Arc::new(Mutex::new(Preview {
            generation: 1,
            manifest: built.manifest,
            shots: built.shots,
            audio_files: built.audio_files,
            changed: Vec::new(),
            error: None,
            building: false,
        }));
        let edited = std::fs::read_to_string(&script).unwrap() + "\nOne more line.\n";
        std::fs::write(&script, edited).unwrap();

        // Until the watcher publishes, with room for a slow machine's
        // recompile: the watcher itself never returns.
        let seen = state.clone();
        let published = async move {
            while seen.lock().unwrap().generation == 1 {
                tokio::time::sleep(POLL / 5).await;
            }
        };
        let watching = watch(state, project, script, "en".into(), compiled);
        tokio::select! {
            () = watching => unreachable!("the watcher returned"),
            r = tokio::time::timeout(std::time::Duration::from_secs(20), published) => {
                r.expect("the edit was never seen");
            }
        }
    }

    /// With no narration there is no audio to read a format from, and the
    /// preview must still publish what `dub` does (#24).
    #[tokio::test]
    async fn a_script_with_no_narration_publishes_the_audio_format_dub_does() {
        let dir = teleprompt_testkit::test_dir("preview-silent");
        crate::cmd::new::scaffold(&dir).unwrap();
        let script = dir.join("scripts/silent.md");
        std::fs::write(&script, "---\nteleprompt: 1\n---\n\n# Silence\n").unwrap();
        let project = Project::discover(&dir).unwrap();

        let served = rebuild(&project, &script, "en", None).await.unwrap();
        let dubbed = match crate::cmd::dub::run_dub(
            &project,
            &script,
            "en",
            &dir.join("out"),
            false,
        )
        .await
        {
            Ok(out) => out,
            Err(crate::cmd::dub::DubError::Validation(e)) => panic!("{e:?}"),
            Err(crate::cmd::dub::DubError::Runtime(e)) => panic!("{e}"),
        };
        assert_eq!(served.manifest.audio, dubbed.manifest.audio);
    }

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
