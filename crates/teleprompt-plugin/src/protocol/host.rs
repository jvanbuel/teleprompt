//! Teleprompt's side: finding plugins, starting them, and holding each as
//! the contract it implements, so the rest of teleprompt cannot tell an
//! outside plugin from a built-in one.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use serde::de::DeserializeOwned;
use serde::Serialize;
use teleprompt_core::{BlockId, Diagnostic, Hash, ShotId};

use super::{
    Body, Capture, Captured, Description, Need, Retime, Retimed, Shots, Validation, PREFIX, VERSION,
};
use crate::capture::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session};
use crate::scene::{BlockSource, SceneCompiler, Shot, Validated};
use crate::ScenePlugin;
use teleprompt_core::tool::Tool;

/// A plugin found on disk, not yet started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub path: PathBuf,
}

/// Where plugins are installed when not on PATH: `$TELEPROMPT_PLUGINS`, or
/// `teleprompt/plugins` in the user's data directory.
pub fn plugins_dir() -> PathBuf {
    crate::dirs::data("TELEPROMPT_PLUGINS", "plugins")
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
            if !out.iter().any(|o| o.name == f.name) {
                out.push(f);
            }
        }
    }
    out
}

/// The plugin `path` is, if its name says it is one and it can be run.
fn plugin_at(path: &Path) -> Option<Found> {
    let file = path.file_name()?.to_str()?;
    let name = file.strip_prefix(PREFIX)?;
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    (valid && executable(path)).then(|| Found {
        name: name.to_string(),
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
    /// it does not answer to its name.
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
                if d.name != self.found.name {
                    return Err(format!(
                        "{} describes itself as `{}`",
                        self.found.path.display(),
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
            Ok(d) => Box::leak(d.needs.iter().map(Need::into_tool).collect()),
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
        let spawn = || {
            Command::new(&self.found.path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
        };
        // A program just written, as by an installer, can be briefly busy
        // while another thread's child still holds it open: ETXTBSY.
        let mut tries = 0;
        let mut child = loop {
            match spawn() {
                Err(e) if e.raw_os_error() == Some(26) && tries < 20 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                other => {
                    break other
                        .map_err(|e| format!("cannot run {}: {e}", self.found.path.display()))?
                }
            }
        };
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
        // A program that has already exited cannot be sent anything, but
        // what it printed first says more than the broken pipe does.
        let sent = writeln!(self.stdin, "{request}").and_then(|()| self.stdin.flush());
        let mut line = String::new();
        loop {
            line.clear();
            let read = self
                .stdout
                .read_line(&mut line)
                .map_err(|e| format!("cannot read its answer to `{method}`: {e}"))?;
            if read == 0 {
                return Err(match &sent {
                    Err(e) => format!("cannot send `{method}`: {e}"),
                    Ok(()) => format!("it stopped before answering `{method}`"),
                });
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

/// An outside scene plugin, as teleprompt registers its built-in ones.
pub fn scene(found: Found) -> ScenePlugin {
    let name: &'static str = Box::leak(found.name.clone().into_boxed_str());
    let plugin = Arc::new(Plugin::new(found));
    ScenePlugin::new(
        ExternalScene {
            plugin: plugin.clone(),
            kind: name,
        },
        ExternalCapture { plugin },
    )
}

/// An outside plugin's scene compiler.
struct ExternalScene {
    plugin: Arc<Plugin>,
    kind: &'static str,
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
        Ok(split
            .shots
            .into_iter()
            .enumerate()
            .map(|(i, part)| {
                let hash = Hash::of_fields(&[self.kind, &part.source]);
                Shot::numbered(block_id, i, part.source, hash).lasting(part.length)
            })
            .collect())
    }

    /// What the plugin answers; a plugin without `retime` answers an
    /// error, which is a shot that cannot be re-timed.
    fn retime(&self, shot: &Shot, target_ms: u64) -> Option<String> {
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
        self.plugin.describe().map_or(true, |d| d.continues)
    }
}

/// An outside plugin's capture backend.
struct ExternalCapture {
    plugin: Arc<Plugin>,
}

impl CaptureBackend for ExternalCapture {
    fn unavailable(&self) -> Option<String> {
        match self.plugin.describe() {
            Err(e) => Some(e),
            Ok(_) => teleprompt_core::tool::missing_of(self.needs()),
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
            session: session.clone(),
            frame: *frame,
            out_dir: out_dir.to_path_buf(),
        };
        let mut progress = |event: serde_json::Value| {
            if let Ok(p) = serde_json::from_value::<Progress>(event) {
                on_progress(Progress {
                    scene: session.scene.clone(),
                    ..p
                });
            }
        };
        let captured: Captured = self
            .plugin
            .call("capture", asked, &mut progress)
            .map_err(failed)?;
        Ok(captured.clips)
    }
}
