//! Teleprompt's side: finding plugins, starting them, and holding each as
//! the contract it implements, so the rest of teleprompt cannot tell an
//! outside plugin from a built-in one.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::de::DeserializeOwned;
use serde::Serialize;
use teleprompt_core::{BlockId, Diagnostic, Hash, ShotId};

use super::{
    AdapterTraits, Body, Capture, Captured, Configure, Configured, Description, Kind, Probed,
    Retime, Retimed, Shots, Spoken, Synthesize, Unavailable, Validation, VoiceTraits, Voices,
    WireFrame, WireProgress, WireSession, WireSessionShot, WireShot, VERSION,
};
use crate::capture::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session};
use crate::scene::{BlockSource, Measured, SceneCompiler, Shot, Validated};
use crate::tool::Tool;
use crate::voice::{
    async_trait, wav, LanguageSupport, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities,
    VoiceError,
};
use crate::Adapter;

/// A plugin found on disk, not yet started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub kind: Kind,
    pub path: PathBuf,
}

/// Where plugins are installed when not on PATH: `$TELEPROMPT_PLUGINS`, or
/// `teleprompt/plugins` in the user's data directory.
pub fn plugins_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TELEPROMPT_PLUGINS") {
        return PathBuf::from(dir);
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    data.join("teleprompt/plugins")
}

/// Every plugin in the plugins directory and on PATH, by its executable's
/// name; the first of a name wins, the plugins directory first. Starts
/// none of them.
pub fn discover() -> Vec<Found> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs = std::iter::once(plugins_dir()).chain(std::env::split_paths(&path));
    let mut out: Vec<Found> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut found: Vec<Found> = entries
            .flatten()
            .filter_map(|e| plugin_at(&e.path()))
            .collect();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        for f in found {
            if !out.iter().any(|o| o.kind == f.kind && o.name == f.name) {
                out.push(f);
            }
        }
    }
    out
}

/// The plugin `path` is, if its name says it is one and it can be run.
fn plugin_at(path: &Path) -> Option<Found> {
    let file = path.file_name()?.to_str()?;
    let (kind, name) = [Kind::Adapter, Kind::Voice]
        .into_iter()
        .find_map(|k| Some((k, file.strip_prefix(k.prefix())?)))?;
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    (valid && executable(path)).then(|| Found {
        name: name.to_string(),
        kind,
        path: path.to_path_buf(),
    })
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.is_file()
}

/// A running plugin, or one to start on first use. Requests go one at a
/// time; the process is stopped when the last handle goes.
pub struct Plugin {
    found: Found,
    conn: Mutex<Option<Conn>>,
    description: OnceLock<Result<Description, String>>,
    needs: OnceLock<&'static [&'static Tool]>,
}

struct Conn {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Plugin {
    pub fn new(found: Found) -> Self {
        Self {
            found,
            conn: Mutex::new(None),
            description: OnceLock::new(),
            needs: OnceLock::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.found.name
    }

    /// What the plugin says it is, asked once; why it cannot be used, when
    /// it does not answer as one of its kind and name.
    pub fn describe(&self) -> Result<&Description, String> {
        self.description
            .get_or_init(|| {
                let d: Description = self.call(
                    "describe",
                    serde_json::json!({ "protocol": VERSION }),
                    &mut |_| {},
                )?;
                if d.protocol != VERSION {
                    return Err(format!(
                        "plugin `{}` speaks protocol {}, and this teleprompt speaks {VERSION}",
                        self.found.name, d.protocol
                    ));
                }
                if d.kind != self.found.kind || d.name != self.found.name {
                    return Err(format!(
                        "{} describes itself as the {:?} `{}`",
                        self.found.path.display(),
                        d.kind,
                        d.name
                    ));
                }
                Ok(d)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// What it needs that teleprompt does not ship, as it describes them.
    pub fn needs(&self) -> &'static [&'static Tool] {
        self.needs.get_or_init(|| match self.describe() {
            Ok(d) => Box::leak(d.needs.iter().map(Tool::from_wire).collect()),
            Err(_) => &[],
        })
    }

    pub fn path(&self) -> &Path {
        &self.found.path
    }

    /// Sends `method`, passing each progress event to `progress`, and
    /// returns the answer.
    pub fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: impl Serialize,
        progress: &mut dyn FnMut(serde_json::Value),
    ) -> Result<T, String> {
        let mut guard = self.conn.lock().map_err(|_| "a plugin call panicked")?;
        if guard.is_none() {
            *guard = Some(self.start()?);
        }
        let conn = guard.as_mut().expect("started");
        let answer = conn.ask(method, params, progress);
        if answer.is_err() && conn.child.try_wait().ok().flatten().is_some() {
            // Gone: the next call starts it again.
            *guard = None;
        }
        let value = answer.map_err(|e| format!("plugin `{}`: {e}", self.found.name))?;
        serde_json::from_value(value).map_err(|e| {
            format!(
                "plugin `{}` answered `{method}` with something else: {e}",
                self.found.name
            )
        })
    }

    fn start(&self) -> Result<Conn, String> {
        let mut child = Command::new(&self.found.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", self.found.path.display()))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        Ok(Conn {
            child,
            stdin,
            stdout,
            next: 0,
        })
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.conn.lock() {
            if let Some(mut conn) = guard.take() {
                let _ = conn.child.kill();
                let _ = conn.child.wait();
            }
        }
    }
}

impl Conn {
    fn ask(
        &mut self,
        method: &str,
        params: impl Serialize,
        progress: &mut dyn FnMut(serde_json::Value),
    ) -> Result<serde_json::Value, String> {
        self.next += 1;
        let id = self.next;
        let request = serde_json::json!({ "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{request}")
            .and_then(|()| self.stdin.flush())
            .map_err(|e| format!("cannot send `{method}`: {e}"))?;
        let mut line = String::new();
        loop {
            line.clear();
            let read = self
                .stdout
                .read_line(&mut line)
                .map_err(|e| format!("cannot read its answer to `{method}`: {e}"))?;
            if read == 0 {
                return Err(format!("it stopped before answering `{method}`"));
            }
            let Ok(mut message) = serde_json::from_str::<serde_json::Value>(&line) else {
                return Err(format!(
                    "it wrote something that is not JSON: {}",
                    line.trim()
                ));
            };
            if message["id"] != id {
                continue;
            }
            if let Some(event) = message.get_mut("progress") {
                progress(event.take());
            } else if let Some(error) = message.get("error") {
                return Err(error
                    .as_str()
                    .map_or_else(|| error.to_string(), String::from));
            } else {
                return Ok(message["result"].take());
            }
        }
    }
}

/// An outside adapter, as teleprompt registers its built-in ones.
pub fn adapter(found: Found) -> Adapter {
    let name: &'static str = Box::leak(found.name.clone().into_boxed_str());
    let plugin = Arc::new(Plugin::new(found));
    Adapter::new(
        ExternalScene {
            plugin: plugin.clone(),
            kind: name,
            timings: Mutex::new(HashMap::new()),
        },
        ExternalCapture { plugin },
    )
}

/// An outside adapter's scene compiler.
struct ExternalScene {
    plugin: Arc<Plugin>,
    kind: &'static str,
    /// What `shots` said each source takes, so `estimate` need not ask.
    timings: Mutex<HashMap<String, Measured>>,
}

impl ExternalScene {
    fn traits(&self) -> AdapterTraits {
        self.plugin
            .describe()
            .ok()
            .and_then(|d| d.adapter.clone())
            .unwrap_or_default()
    }
}

fn measured(shot: &WireShot) -> Measured {
    match (shot.ms, shot.exact) {
        (Some(ms), true) => Measured::Exact(ms),
        (Some(ms), false) => Measured::Estimated(ms),
        (None, _) => Measured::Unknown,
    }
}

impl SceneCompiler for ExternalScene {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        let body = Body {
            scene: src.scene.clone(),
            body: src.body.clone(),
        };
        let checked: Validation = self
            .plugin
            .call("validate", body, &mut |_| {})
            .map_err(|e| vec![Diagnostic::error(e)])?;
        if checked.errors.is_empty() {
            return Ok(Validated::from(src));
        }
        Err(checked
            .errors
            .into_iter()
            .map(|e| {
                let mut d = Diagnostic::error(e.message);
                if let Some(help) = e.help {
                    d = d.with_help(help);
                }
                match e.line {
                    Some(i) => {
                        let len = src.body.lines().nth(i).map_or(1, |l| l.len().max(1));
                        src.origin.locate(d, i, len)
                    }
                    None => d,
                }
            })
            .collect())
    }

    fn shots(&self, v: &Validated, block_id: &BlockId) -> Result<Vec<Shot>, Vec<Diagnostic>> {
        let body = Body {
            scene: v.scene.clone(),
            body: v.body.clone(),
        };
        let split: Shots = self
            .plugin
            .call("shots", body, &mut |_| {})
            .map_err(|e| vec![Diagnostic::error(e)])?;
        let mut timings = self.timings.lock().expect("not poisoned");
        Ok(split
            .shots
            .into_iter()
            .enumerate()
            .map(|(i, w)| {
                timings.insert(w.source.clone(), measured(&w));
                let hash = Hash::of_fields(&[self.kind, &w.source]);
                Shot::numbered(block_id, i, w.source, hash)
            })
            .collect())
    }

    fn estimate(&self, shot: &Shot) -> Measured {
        if let Some(m) = self.timings.lock().expect("not poisoned").get(&shot.source) {
            return *m;
        }
        let asked = WireShot {
            source: shot.source.clone(),
            ..WireShot::default()
        };
        self.plugin
            .call::<WireShot>("estimate", asked, &mut |_| {})
            .map_or(Measured::Unknown, |w| measured(&w))
    }

    fn retime(&self, shot: &Shot, target_ms: u64) -> Option<String> {
        if !self.traits().retimes {
            return None;
        }
        let asked = Retime {
            source: shot.source.clone(),
            target_ms,
        };
        self.plugin
            .call::<Retimed>("retime", asked, &mut |_| {})
            .ok()?
            .source
    }

    fn continues(&self) -> bool {
        self.traits().continues
    }
}

/// An outside adapter's capture backend.
struct ExternalCapture {
    plugin: Arc<Plugin>,
}

impl CaptureBackend for ExternalCapture {
    fn unavailable(&self) -> Option<String> {
        if let Err(e) = self.plugin.describe() {
            return Some(e);
        }
        match self
            .plugin
            .call::<Unavailable>("unavailable", (), &mut |_| {})
        {
            Ok(u) => u.reason,
            Err(e) => Some(e),
        }
    }

    fn needs(&self) -> &'static [&'static Tool] {
        self.plugin.needs()
    }

    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Clip>, CaptureError> {
        let failed = |reason: String| CaptureError::Failed {
            backend: self.plugin.name().to_string(),
            shot: session
                .shots
                .iter()
                .find(|s| s.wanted)
                .or(session.shots.first())
                .map_or_else(|| ShotId::new(&session.scene), |s| s.id.clone()),
            reason,
        };
        let asked = Capture {
            session: WireSession {
                scene: session.scene.clone(),
                name: session.name.clone(),
                settings: session.settings.clone(),
                shots: session
                    .shots
                    .iter()
                    .map(|s| WireSessionShot {
                        id: s.id.to_string(),
                        key: s.key.to_string(),
                        source: s.source.clone(),
                        duration_ms: s.duration_ms,
                        wanted: s.wanted,
                    })
                    .collect(),
            },
            frame: WireFrame {
                width: frame.width,
                height: frame.height,
                fps: frame.fps,
            },
            out_dir: out_dir.to_path_buf(),
        };
        let mut progress = |event: serde_json::Value| {
            if let Ok(p) = serde_json::from_value::<WireProgress>(event) {
                on_progress(Progress {
                    scene: session.scene.clone(),
                    shot: ShotId::new(p.shot),
                    done: p.done,
                    of: p.of,
                });
            }
        };
        let captured: Captured = self
            .plugin
            .call("capture", asked, &mut progress)
            .map_err(failed)?;
        captured
            .clips
            .into_iter()
            .map(|c| {
                let key = serde_json::from_value(serde_json::Value::String(c.key.clone()))
                    .map_err(|_| failed(format!("a clip's key is not a hash: {}", c.key)))?;
                Ok(Clip { key, path: c.path })
            })
            .collect()
    }
}

/// An outside voice, as teleprompt registers its built-in ones. It starts,
/// and is given `settings`, only when first asked for something.
pub fn voice(found: Found, settings: Option<serde_json::Value>) -> ExternalVoice {
    ExternalVoice {
        plugin: Arc::new(Plugin::new(found)),
        settings,
        configured: OnceLock::new(),
        lines: AtomicU64::new(0),
    }
}

pub struct ExternalVoice {
    plugin: Arc<Plugin>,
    settings: Option<serde_json::Value>,
    /// Its version, once configured.
    configured: OnceLock<Result<String, String>>,
    /// How many lines it has been asked for, to name each one's file.
    lines: AtomicU64,
}

impl ExternalVoice {
    pub fn plugin(&self) -> &Plugin {
        &self.plugin
    }

    fn traits(&self) -> VoiceTraits {
        self.plugin
            .describe()
            .ok()
            .and_then(|d| d.voice.clone())
            .unwrap_or_default()
    }

    /// Its version, giving it its settings first if not yet given.
    fn configure(&self) -> Result<&str, VoiceError> {
        self.configured
            .get_or_init(|| {
                self.plugin.describe()?;
                let asked = Configure {
                    settings: self.settings.clone(),
                };
                self.plugin
                    .call::<Configured>("configure", asked, &mut |_| {})
                    .map(|c| c.version)
            })
            .as_deref()
            .map_err(|e| VoiceError::Other(e.clone()))
    }
}

#[async_trait]
impl VoiceBackend for ExternalVoice {
    fn id(&self) -> &str {
        self.plugin.name()
    }

    fn capabilities(&self) -> VoiceCapabilities {
        let traits = self.traits();
        VoiceCapabilities {
            languages: LanguageSupport::Any,
            cloning: false,
            cross_lingual: false,
            word_timings: traits.word_timings,
            ssml: false,
            speed_control: traits.speed_control,
            // A voice that cannot be configured keys nothing it could make.
            version: match self.configure() {
                Ok(v) => format!("{}:{v}", self.plugin.name()),
                Err(e) => format!("unusable: {e}"),
            },
        }
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        self.configure()?;
        let n = self.lines.fetch_add(1, Ordering::SeqCst);
        let out = std::env::temp_dir().join(format!(
            "teleprompt-{}-{}-{n}.wav",
            self.plugin.name(),
            std::process::id()
        ));
        let asked = Synthesize {
            text: req.text.clone(),
            locale: req.locale.clone(),
            voice: req.voice.clone(),
            speed: req.speed,
            instruct: req.instruct.clone(),
            out: out.clone(),
        };
        let spoken = self.plugin.call::<Spoken>("synthesize", asked, &mut |_| {});
        let bytes = std::fs::read(&out);
        let _ = std::fs::remove_file(&out);
        let spoken = spoken.map_err(VoiceError::Other)?;
        let bytes = bytes.map_err(|e| {
            VoiceError::Other(format!(
                "plugin `{}` wrote no audio to {}: {e}",
                self.plugin.name(),
                out.display()
            ))
        })?;
        let pcm = wav::decode(&bytes).map_err(VoiceError::Other)?;
        Ok(Synthesized {
            pcm,
            word_timings: spoken.word_timings.filter(|_| self.traits().word_timings),
        })
    }

    fn address(&self) -> Option<String> {
        self.traits().address
    }

    async fn voices(&self) -> Option<Result<Vec<String>, VoiceError>> {
        if !self.traits().lists_voices {
            return None;
        }
        Some(
            self.configure()
                .and_then(|_| {
                    self.plugin
                        .call::<Voices>("voices", (), &mut |_| {})
                        .map_err(VoiceError::Other)
                })
                .map(|v| v.voices),
        )
    }

    async fn probe(&self) -> Result<String, VoiceError> {
        if !self.traits().probes {
            return Err(self.unsupported("a probe"));
        }
        self.configure()?;
        self.plugin
            .call::<Probed>("probe", (), &mut |_| {})
            .map(|p| p.line)
            .map_err(VoiceError::Other)
    }
}
