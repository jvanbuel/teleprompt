//! Recording a browser scene by handing Playwright its own script.
//!
//! A session's shots are concatenated into one script and run once, so
//! shot *n* opens on the page shot *n-1* left. Playwright records a video
//! and [`teleprompt_capture::reel`] cuts it into a clip per shot. Nothing
//! here interprets the author's script; it is only wrapped.

use std::path::Path;
use std::process::{Command, Stdio};

use teleprompt_capture::reel::{cut, starved, windows};
use teleprompt_capture::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session, WorkDir};

/// How long an action annotation (pointer, click ripple, title) stays on
/// screen, unless the scene sets `annotation_ms`.
///
/// Playwright's default of 500ms is too brief to see in a video watched
/// once. The annotation is not free: Playwright holds each action this long
/// before the next runs (four clicks take 1.4s at 300ms, 10.2s at 2500ms),
/// so it spends the narration's budget. 1200ms is plainly visible and still
/// fits about half a dozen clicks under one sentence; a busier scene should
/// lower it rather than overrun its slot.
const DEFAULT_ANNOTATION_MS: u32 = 1_200;

/// The action title's font size. Playwright's 24px suits a trace viewer,
/// not a video that is scaled down wherever it is watched.
const DEFAULT_ANNOTATION_SIZE: u32 = 32;

/// The script teleprompt writes around a session's shots.
///
/// Each shot is padded to its scheduled length by holding the page still
/// afterwards; the author's code is not re-timed. A shot that overruns is
/// not padded, and [`starved`] catches a reel that falls too far behind.
pub fn script_for(session: &Session, frame: &Frame, video_dir: &str) -> String {
    let mut out = String::new();
    out.push_str("import { chromium } from 'playwright';\n\n");
    out.push_str("const browser = await chromium.launch({\n");
    if let Some(bin) = session.settings.get("executable") {
        out.push_str(&format!("  executablePath: {},\n", quote(bin)));
    }
    out.push_str("  args: ['--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu'],\n");
    out.push_str("});\n");
    out.push_str("const context = await browser.newContext({\n");
    out.push_str(&format!(
        "  viewport: {{ width: {}, height: {} }},\n",
        frame.width, frame.height
    ));
    out.push_str("  recordVideo: {\n");
    out.push_str(&format!("    dir: {},\n", quote(video_dir)));
    out.push_str(&format!(
        "    size: {{ width: {}, height: {} }},\n",
        frame.width, frame.height
    ));
    // Without a visible pointer a click is a page changing by itself.
    // See `DEFAULT_ANNOTATION_MS` for what `duration` costs.
    out.push_str("    showActions: {\n");
    out.push_str(&format!(
        "      duration: {},\n",
        session
            .settings
            .get("annotation_ms")
            .map_or(DEFAULT_ANNOTATION_MS.to_string(), String::clone)
    ));
    out.push_str(&format!(
        "      fontSize: {},\n",
        session
            .settings
            .get("annotation_size")
            .map_or(DEFAULT_ANNOTATION_SIZE.to_string(), String::clone)
    ));
    out.push_str(&format!(
        "      cursor: {},\n",
        quote(
            session
                .settings
                .get("cursor")
                .map_or("pointer", String::as_str)
        )
    ));
    out.push_str(&format!(
        "      position: {},\n",
        quote(
            session
                .settings
                .get("labels")
                .map_or("bottom-right", String::as_str)
        )
    ));
    out.push_str("    },\n  },\n});\n");
    out.push_str("const page = await context.newPage();\n\n");

    for (shot, (_, duration_ms)) in session.shots.iter().zip(windows(session)) {
        out.push_str(&format!("// {}\n{{\n", shot.id));
        out.push_str("  const __began = Date.now();\n");
        for line in shot.source.trim_end().lines() {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&format!(
            "  const __left = {duration_ms} - (Date.now() - __began);\n"
        ));
        out.push_str("  if (__left > 0) await page.waitForTimeout(__left);\n}\n\n");
    }

    out.push_str("await context.close();\nawait browser.close();\n");
    out
}

/// A JavaScript string literal.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Records `playwright` scenes by running the author's script.
#[derive(Debug, Clone)]
pub struct PlaywrightRender {
    pub node: String,
    pub ffmpeg: String,
}

impl Default for PlaywrightRender {
    fn default() -> Self {
        Self {
            node: "node".into(),
            ffmpeg: "ffmpeg".into(),
        }
    }
}

impl CaptureBackend for PlaywrightRender {
    fn id(&self) -> &'static str {
        "playwright"
    }

    fn adapter(&self) -> &'static str {
        "playwright"
    }

    fn unavailable(&self) -> Option<String> {
        let mut missing = Vec::new();
        if !crate::on_path(&self.node) {
            missing.push(self.node.clone());
        }
        if !crate::on_path(&self.ffmpeg) {
            missing.push(self.ffmpeg.clone());
        }
        (!missing.is_empty()).then(|| format!("{} is not on PATH", missing.join(" and ")))
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
            .first()
            .map(|s| s.id.clone())
            .unwrap_or_default();
        let failed = |shot: &str, why: String| CaptureError::Failed {
            backend: "playwright".into(),
            shot: shot.to_string(),
            reason: why,
        };

        let work = WorkDir::create(out_dir, "pw")
            .map_err(|e| failed(&first, format!("{}: {e}", out_dir.display())))?;
        let video_dir = work.join("video");
        let script = work.join("session.mjs");
        std::fs::write(
            &script,
            script_for(session, frame, &video_dir.display().to_string()),
        )
        .map_err(|e| failed(&first, format!("{}: {e}", script.display())))?;

        let ran = Command::new(&self.node)
            .arg(&script)
            .current_dir(&work)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| failed(&first, format!("{} could not be run: {e}", self.node)))?;
        if !ran.status.success() {
            let said = String::from_utf8_lossy(&ran.stderr);
            let tail: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).collect();
            let from = tail.len().saturating_sub(4);
            return Err(failed(
                &first,
                format!(
                    "the script exited {}: {}",
                    ran.status,
                    tail[from..].join(" / ")
                ),
            ));
        }

        // Playwright names the file itself, after the page that made it.
        let video = std::fs::read_dir(&video_dir)
            .ok()
            .and_then(|entries| {
                entries
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.extension().is_some_and(|e| e == "webm"))
            })
            .ok_or_else(|| {
                failed(
                    &first,
                    format!(
                        "the script exited 0 and left no recording in {}",
                        video_dir.display()
                    ),
                )
            })?;

        if let Some(why) = starved(&video, session) {
            return Err(failed(&first, why));
        }

        let wanted = session.wanted();
        let mut clips = Vec::new();
        for (shot, (from_ms, duration_ms)) in session.shots.iter().zip(windows(session)) {
            if !shot.wanted {
                continue;
            }
            let clip = out_dir.join(format!("{}.mp4", shot.key));
            cut(&self.ffmpeg, &video, &clip, from_ms, duration_ms)
                .map_err(|reason| failed(&shot.id, reason))?;
            clips.push(Clip {
                key: shot.key,
                path: clip,
            });
            on_progress(Progress {
                scene: session.scene.clone(),
                shot: shot.id.clone(),
                done: clips.len(),
                of: wanted,
            });
        }

        Ok(clips)
    }
}
