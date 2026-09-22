//! Rendering a motion scene by handing Remotion a composition per shot.
//!
//! Every shot becomes one `<Composition>` whose `durationInFrames` is the
//! slot the schedule gave it, and one Node process bundles them once and
//! renders each to its own clip. There is no reel to cut: a composition
//! already *is* the length of its shot, so the clip Remotion writes is the
//! clip the renderer places.
//!
//! That also changes what a session means. A terminal shot opens on the
//! screen the one before it left behind, so a cached shot still has to
//! run. A composition is a function of its own frame and nothing else —
//! shot *n* does not open on shot *n-1* — so a shot whose clip is cached
//! is not rendered at all.
//!
//! Nothing here interprets the JSX. Remotion's bundler compiles it and
//! reports what is wrong with it; the only thing teleprompt has to know is
//! how to wrap it.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use teleprompt_capture::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session};

/// Names a shot may use without the project exporting them: Remotion's
/// own functions and hooks. Its components — `AbsoluteFill`, `Sequence`,
/// `Img` — are capitalised and found the way the project's are.
const REMOTION_FUNCTIONS: &[&str] = &[
    "interpolate",
    "interpolateColors",
    "random",
    "spring",
    "staticFile",
    "useCurrentFrame",
    "useVideoConfig",
];

/// How many frames `ms` is at `fps`, and never none: Remotion refuses a
/// composition of zero frames, and a shot the schedule kept deserves one.
pub fn frames(ms: u64, fps: u32) -> u64 {
    ((ms * u64::from(fps) + 500) / 1000).max(1)
}

/// The composition id for the `index`th rendered shot.
///
/// Positional rather than derived from the shot id, because Remotion
/// allows only letters, digits and dashes in an id and a shot id is
/// `block#index`.
fn composition_id(index: usize) -> String {
    format!("shot-{index}")
}

/// The entry point teleprompt writes for a session: one composition per
/// shot to render, registered with Remotion.
///
/// `components` is the module a shot's capitalised names are looked up
/// in, as an import specifier — the project's own components. Remotion's
/// exports are searched after it, so a project may shadow one.
pub fn entry_for(session: &Session, frame: &Frame, components: Option<&str>) -> String {
    let background = session.setting("background", "black");
    let mut out = String::new();
    out.push_str("import React from 'react';\n");
    out.push_str("import { registerRoot, Composition, AbsoluteFill } from 'remotion';\n");
    out.push_str("import * as __remotion from 'remotion';\n");
    match components {
        Some(module) => out.push_str(&format!("import * as __components from {};\n", js(module))),
        None => out.push_str("const __components = {};\n"),
    }
    out.push_str("const __scope = { ...__remotion, ...__components };\n");
    out.push_str(&format!(
        "const {{ {} }} = __remotion;\n\n",
        REMOTION_FUNCTIONS.join(", ")
    ));

    let rendered: Vec<_> = session.shots.iter().filter(|s| s.wanted).collect();
    for (index, shot) in rendered.iter().enumerate() {
        out.push_str(&format!("// {}\nconst Shot{index} = () => {{\n", shot.id));
        // A shot names components as bare JSX tags, so each one it uses is
        // brought into scope by name. Names neither module exports fall
        // back to the global of the same name — `Math`, `Date` — and
        // otherwise to `undefined`, which React reports by name when it is
        // rendered.
        for name in capitalised(&shot.source)
            .into_iter()
            .filter(|n| n != "React")
        {
            out.push_str(&format!(
                "  const {name} = __scope[{q}] ?? globalThis[{q}];\n",
                q = js(&name)
            ));
        }
        out.push_str(&format!(
            "  return (\n    <AbsoluteFill style={{{{ background: {} }}}}>\n      <>\n",
            js(background)
        ));
        for line in shot.source.trim_end().lines() {
            out.push_str("        ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("      </>\n    </AbsoluteFill>\n  );\n};\n\n");
    }

    out.push_str("const Root = () => (\n  <>\n");
    for (index, shot) in rendered.iter().enumerate() {
        out.push_str(&format!(
            "    <Composition id={} component={{Shot{index}}} durationInFrames={{{}}} \
             fps={{{}}} width={{{}}} height={{{}}} />\n",
            js(&composition_id(index)),
            frames(shot.duration_ms, frame.fps),
            frame.fps,
            frame.width,
            frame.height,
        ));
    }
    out.push_str("  </>\n);\n\nregisterRoot(Root);\n");
    out
}

/// The Node script that bundles `entry` once and renders each shot to
/// its clip, saying so on stdout as each one lands.
pub fn render_script_for(
    session: &Session,
    entry: &Path,
    project: &Path,
    out_dir: &Path,
) -> String {
    let jobs: Vec<serde_json::Value> = session
        .shots
        .iter()
        .filter(|s| s.wanted)
        .enumerate()
        .map(|(index, shot)| {
            serde_json::json!({
                "id": composition_id(index),
                "shot": shot.id,
                "out": out_dir.join(format!("{}.mp4", shot.key)).display().to_string(),
            })
        })
        .collect();
    let browser = browser(session).map_or_else(|| "null".to_string(), |b| js(&b));

    let mut out = String::new();
    out.push_str("import { bundle } from '@remotion/bundler';\n");
    out.push_str("import { renderMedia, selectComposition } from '@remotion/renderer';\n\n");
    out.push_str(&format!(
        "const jobs = {};\n",
        serde_json::Value::Array(jobs)
    ));
    out.push_str(&format!("const browserExecutable = {browser};\n"));
    out.push_str(&format!(
        "const serveUrl = await bundle({{ entryPoint: {}, rootDir: {} }});\n\n",
        js(&entry.display().to_string()),
        js(&project.display().to_string()),
    ));
    out.push_str("for (const job of jobs) {\n");
    out.push_str(
        "  const composition = await selectComposition({ serveUrl, id: job.id, \
         browserExecutable });\n",
    );
    out.push_str(
        "  await renderMedia({ composition, serveUrl, codec: 'h264', \
         outputLocation: job.out, browserExecutable, logLevel: 'error' });\n",
    );
    out.push_str("  process.stdout.write(JSON.stringify({ rendered: job.shot }) + '\\n');\n");
    out.push_str("}\n");
    out
}

/// The Chrome to render with: the scene's `browser` setting, else
/// `TELEPROMPT_REMOTION_BROWSER`, else none — in which case Remotion
/// downloads its own headless shell the first time it renders.
///
/// The variable is for a machine rather than a project: a CI image with a
/// browser already installed, or a network that will not let Remotion
/// fetch one. A path that differs from machine to machine does not belong
/// in a committed `teleprompt.toml`.
fn browser(session: &Session) -> Option<String> {
    session
        .settings
        .get("browser")
        .cloned()
        .or_else(|| std::env::var("TELEPROMPT_REMOTION_BROWSER").ok())
        .filter(|b| !b.is_empty())
}

/// Every capitalised identifier in `source`, once each, in a stable order.
///
/// A superset of the components a shot uses — words in its text are
/// capitalised too — and that is harmless: a name bound and never
/// rendered costs a lookup.
fn capitalised(source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut word = String::new();
    let mut previous: Option<char> = None;
    for c in source.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            word.push(c);
            continue;
        }
        // `props.Title` is a property, not a name in scope.
        let member = previous == Some('.');
        if !member && word.starts_with(|c: char| c.is_ascii_uppercase()) {
            names.insert(std::mem::take(&mut word));
        }
        word.clear();
        previous = Some(c);
    }
    names
}

/// A JavaScript string literal.
fn js(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

/// Records `remotion` scenes by rendering the author's JSX.
#[derive(Debug, Clone)]
pub struct RemotionRender {
    pub node: String,
}

impl Default for RemotionRender {
    fn default() -> Self {
        Self {
            node: "node".into(),
        }
    }
}

impl CaptureBackend for RemotionRender {
    fn id(&self) -> &'static str {
        "remotion"
    }

    fn adapter(&self) -> &'static str {
        "remotion"
    }

    fn unavailable(&self) -> Option<String> {
        (!crate::on_path(&self.node)).then(|| format!("{} is not on PATH", self.node))
    }

    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Clip>, CaptureError> {
        let first = session
            .shots
            .iter()
            .find(|s| s.wanted)
            .map(|s| s.id.clone())
            .unwrap_or_default();
        let failed = |shot: &str, why: String| CaptureError::Failed {
            backend: "remotion".into(),
            shot: shot.to_string(),
            reason: why,
        };

        // The project is where Remotion is installed, and the generated
        // files are written inside it so that Node and the bundler find
        // its `node_modules` the ordinary way, by walking up.
        let project = absolute(Path::new(session.setting("project", ".")));
        if !project.join("node_modules/@remotion/renderer").is_dir()
            || !project.join("node_modules/@remotion/bundler").is_dir()
        {
            return Err(CaptureError::Unavailable {
                backend: "remotion".into(),
                reason: format!(
                    "{} has no @remotion/renderer and @remotion/bundler installed; \
                     run `npm install` there, or point `project` at the directory \
                     that has them",
                    project.display()
                ),
            });
        }
        let components = match session.settings.get("components") {
            Some(path) => {
                let module = project.join(path);
                if !module.is_file() {
                    return Err(failed(
                        &first,
                        format!(
                            "`components` names {}, which is not a file",
                            module.display()
                        ),
                    ));
                }
                Some(module.display().to_string())
            }
            None => None,
        };
        let out_dir = absolute(out_dir);

        let work = project.join(format!(".teleprompt-remotion-{}", std::process::id()));
        std::fs::create_dir_all(&work)
            .map_err(|e| failed(&first, format!("{}: {e}", work.display())))?;
        let entry = work.join("index.jsx");
        let script = work.join("render.mjs");
        let written = std::fs::write(&entry, entry_for(session, frame, components.as_deref()))
            .and_then(|()| {
                std::fs::write(
                    &script,
                    render_script_for(session, &entry, &project, &out_dir),
                )
            });
        if let Err(e) = written {
            let _ = std::fs::remove_dir_all(&work);
            return Err(failed(&first, format!("{}: {e}", work.display())));
        }

        let result = self.run(session, &script, &project, &out_dir, on_progress);
        let _ = std::fs::remove_dir_all(&work);
        result.map_err(|(shot, why)| failed(shot.as_deref().unwrap_or(&first), why))
    }
}

impl RemotionRender {
    /// Run the render script, reporting each clip as Node announces it.
    fn run(
        &self,
        session: &Session,
        script: &Path,
        project: &Path,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Clip>, (Option<String>, String)> {
        let mut child = Command::new(&self.node)
            .arg(script)
            .current_dir(project)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| (None, format!("{} could not be run: {e}", self.node)))?;

        // Drained on its own thread: a bundler that writes more warnings
        // than a pipe holds would otherwise block on a reader that is
        // waiting on stdout.
        let mut stderr = child.stderr.take().expect("piped");
        let said = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });

        let wanted = session.wanted();
        let mut clips = Vec::new();
        let stdout = child.stdout.take().expect("piped");
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let Some(id) = message.get("rendered").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(shot) = session.shots.iter().find(|s| s.id == id) else {
                continue;
            };
            let path = out_dir.join(format!("{}.mp4", shot.key));
            if !path.is_file() {
                return Err((
                    Some(shot.id.clone()),
                    format!("Remotion reported a render and left no {}", path.display()),
                ));
            }
            clips.push(Clip {
                key: shot.key,
                path,
            });
            on_progress(Progress {
                scene: session.scene.clone(),
                shot: shot.id.clone(),
                done: clips.len(),
                of: wanted,
            });
        }

        let status = child
            .wait()
            .map_err(|e| (None, format!("{} did not finish: {e}", self.node)))?;
        let said = said.join().unwrap_or_default();
        if !status.success() {
            let tail: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).collect();
            let from = tail.len().saturating_sub(6);
            // The shot that failed is the first one Node did not announce.
            let at = session
                .shots
                .iter()
                .filter(|s| s.wanted)
                .nth(clips.len())
                .map(|s| s.id.clone());
            return Err((
                at,
                format!("the render exited {status}: {}", tail[from..].join(" / ")),
            ));
        }
        if clips.len() != wanted {
            return Err((
                None,
                format!(
                    "the render exited 0 having rendered {} of {wanted} shot(s)",
                    clips.len()
                ),
            ));
        }
        Ok(clips)
    }
}

/// `path` made absolute against the directory teleprompt runs in, since
/// the render runs somewhere else.
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}
