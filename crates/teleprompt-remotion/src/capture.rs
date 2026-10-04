//! Rendering a motion scene with the project's own Remotion install.
//!
//! One Node process bundles the project's entry point once and renders
//! each missing shot straight to its clip, at the length the schedule gave
//! it. There is no reel to cut, and a cached shot is not rendered at all:
//! a composition does not open on the screen the one before it left.

use std::path::{Path, PathBuf};
use std::process::Command;

use teleprompt_plugin::capture::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session};

use crate::scene::parse;

const RENDER_SCRIPT: &str = include_str!("render.mjs");

/// How many frames `ms` is at `fps`, rounded, and never none.
pub fn frames(ms: u64, fps: u32) -> u64 {
    teleprompt_core::time::frames(ms, fps).max(1)
}

/// The job `render.mjs` reads: what to bundle, and each wanted shot's
/// composition, props, length and clip path.
pub fn job_for(
    session: &Session,
    frame: &Frame,
    entry: &Path,
    out_dir: &Path,
) -> Result<serde_json::Value, (String, String)> {
    let mut shots = Vec::new();
    for shot in session.shots.iter().filter(|s| s.wanted) {
        let call = parse(&shot.source).ok().flatten().ok_or_else(|| {
            (
                shot.id.to_string(),
                "the shot names no composition".to_string(),
            )
        })?;
        shots.push(serde_json::json!({
            "composition": call.composition,
            "props": call.props,
            "frames": frames(shot.duration_ms, frame.fps),
            "out": out_dir.join(format!("{}.mp4", shot.key)),
        }));
    }
    Ok(serde_json::json!({
        "entry": entry,
        // The scene's `browser`, else the machine's: a path that differs
        // from machine to machine does not belong in a committed config.
        // Neither means Remotion downloads its own headless shell.
        "browser": session.settings.get("browser").cloned()
            .or_else(|| std::env::var("TELEPROMPT_REMOTION_BROWSER").ok()),
        "fps": frame.fps,
        "width": frame.width,
        "height": frame.height,
        "shots": shots,
    }))
}

/// Records `remotion` scenes by rendering the project's compositions.
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
    fn unavailable(&self) -> Option<String> {
        teleprompt_plugin::tool::missing(&[&self.node])
    }

    fn needs(&self) -> &'static [&'static teleprompt_plugin::tool::Tool] {
        static NEEDS: &[&teleprompt_plugin::tool::Tool] =
            &[&teleprompt_plugin::tool::NODE, &crate::tools::REMOTION];
        NEEDS
    }

    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Clip>, CaptureError> {
        let wanted: Vec<_> = session.shots.iter().filter(|s| s.wanted).collect();
        let first = wanted.first().map(|s| s.id.to_string()).unwrap_or_default();
        let failed = |shot: &str, reason: String| CaptureError::Failed {
            backend: "remotion".into(),
            shot: shot.into(),
            reason,
        };

        let project = absolute(&session.path("project", "."));
        if !project.join("node_modules/@remotion/renderer").is_dir()
            || !project.join("node_modules/@remotion/bundler").is_dir()
        {
            return Err(CaptureError::Unavailable {
                backend: "remotion".into(),
                reason: format!(
                    "{} has no @remotion/renderer and @remotion/bundler; run `npm install` there",
                    project.display()
                ),
            });
        }
        let entry = project.join(session.setting("entry", "src/index.ts"));
        let out_dir = absolute(out_dir);
        let job = job_for(session, frame, &entry, &out_dir).map_err(|(s, why)| failed(&s, why))?;

        // Inside the project, so Node finds its node_modules by walking up.
        let work = project.join(format!(".teleprompt-remotion-{}", std::process::id()));
        let script = work.join("render.mjs");
        let job_file = work.join("job.json");
        let ran = std::fs::create_dir_all(&work)
            .and_then(|()| std::fs::write(&script, RENDER_SCRIPT))
            .and_then(|()| std::fs::write(&job_file, job.to_string()))
            .map_err(|e| failed(&first, format!("{}: {e}", work.display())))
            .and_then(|()| {
                teleprompt_plugin::tool::run(
                    Command::new(&self.node)
                        .arg(&script)
                        .arg(&job_file)
                        .current_dir(&project),
                    "the render",
                    6,
                )
                .map_err(|why| failed(&first, why))
            });
        let _ = std::fs::remove_dir_all(&work);
        ran?;

        let mut clips = Vec::new();
        for shot in wanted {
            let path = out_dir.join(format!("{}.mp4", shot.key));
            if !path.is_file() {
                return Err(failed(
                    &shot.id,
                    format!("the render left no {}", path.display()),
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
                of: session.wanted(),
            });
        }
        Ok(clips)
    }
}

/// `path` made absolute, since the render runs in another directory.
fn absolute(path: &Path) -> PathBuf {
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}
