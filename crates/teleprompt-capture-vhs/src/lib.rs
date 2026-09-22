//! Recording a terminal scene, with no browser in it.
//!
//! The tape language is [`teleprompt_scene_vhs`]'s — the same parser the
//! compiler measures with, so a line the two would disagree about cannot
//! exist. What runs it is a pty; what draws it is `agg`, asciinema's
//! renderer, which turns a recording into frames without a display, a font
//! server or a browser.
//!
//! VHS itself is not shelled out to, on purpose. It drives a terminal
//! through `ttyd` and screenshots xterm.js canvases in headless Chromium:
//! in a container that captures zero frames, never invokes an encoder, and
//! exits 0 — a failure with no error in it — and it puts a browser behind
//! a tool whose README says it needs none.

mod cast;
mod session;
pub mod tape;

use std::path::Path;
use std::process::{Command, Stdio};

use teleprompt_capture::{CaptureBackend, CaptureError, Frame, Progress, Session, Shot};

pub use session::{record, Recording, Terminal};

/// The terminal a scene asks for.
///
/// Read from the scene, never from the tape: a tape that picked its own
/// shell or its own size would be picking one teleprompt is not driving,
/// which is the same reason `Set Shell` is refused at compile time. The
/// tape says what to type.
pub fn terminal_for(session: &Session) -> Terminal {
    Terminal {
        cols: session.number("columns", 100) as u16,
        rows: session.number("rows", 30) as u16,
        shell: session.setting("shell", "bash").to_string(),
        settle_ms: u64::from(session.number("settle_ms", 800)),
        // `scene.<name>.env` — a tape that runs the tool it is documenting
        // needs it on `PATH`, and the shell a capture opens is not the one
        // the author has in front of them.
        env: session
            .nested("env")
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        // Where a shell opens matters to a tape: a relative path in one is
        // relative to something. `portable-pty` starts a command in the
        // home directory rather than inheriting, which is not what any
        // other subprocess does and not what an author who wrote
        // `cargo run -- build` from their checkout expects — so the
        // default is the directory the build was run from, and `cwd` on
        // the scene overrides it.
        cwd: session.settings.get("cwd").cloned().or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        }),
    }
}

/// A session's spans, as things to do to a terminal.
pub fn steps_of(session: &Session) -> Vec<Vec<tape::Step>> {
    session
        .steps
        .iter()
        .map(|s| tape::steps(&s.source))
        .collect()
}

/// Records `terminal` scenes by running their tapes against a pty.
#[derive(Debug, Clone)]
pub struct VhsCapture {
    /// asciinema's renderer, which draws a recording into frames.
    pub agg: String,
    /// The encoder, which turns those frames into a clip.
    pub ffmpeg: String,
}

impl Default for VhsCapture {
    fn default() -> Self {
        Self {
            agg: "agg".into(),
            ffmpeg: "ffmpeg".into(),
        }
    }
}

fn on_path(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

impl CaptureBackend for VhsCapture {
    fn id(&self) -> &'static str {
        "pty"
    }

    fn adapter(&self) -> &'static str {
        "vhs"
    }

    fn unavailable(&self) -> Option<String> {
        let missing: Vec<&str> = [self.agg.as_str(), self.ffmpeg.as_str()]
            .into_iter()
            .filter(|p| !on_path(p))
            .collect();
        (!missing.is_empty()).then(|| format!("{} is not on PATH", missing.join(" and ")))
    }

    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Shot>, CaptureError> {
        if let Some(reason) = self.unavailable() {
            return Err(CaptureError::Unavailable {
                backend: self.id().to_string(),
                reason,
            });
        }
        std::fs::create_dir_all(out_dir).map_err(|source| CaptureError::Io {
            path: out_dir.display().to_string(),
            source,
        })?;

        let terminal = terminal_for(session);
        let spans = steps_of(session);

        // Before the terminal opens, not after: `Require` exists so that a
        // missing tool is an error rather than a correctly-timed video of
        // `command not found`.
        let path = terminal
            .env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        let missing = tape::missing(&spans, &path);
        if !missing.is_empty() {
            return Err(CaptureError::Failed {
                backend: self.id().to_string(),
                span: session
                    .steps
                    .first()
                    .map(|s| s.span.clone())
                    .unwrap_or_default(),
                reason: format!(
                    "the tape requires {}, which is not on the scene's PATH",
                    missing.join(" and ")
                ),
            });
        }
        let recording = record(&terminal, &spans).map_err(|e| CaptureError::Failed {
            backend: self.id().to_string(),
            span: session
                .steps
                .first()
                .map(|s| s.span.clone())
                .unwrap_or_default(),
            reason: e.to_string(),
        })?;

        let work = out_dir.join(format!(".pty-{}", std::process::id()));
        std::fs::create_dir_all(&work).map_err(|source| CaptureError::Io {
            path: work.display().to_string(),
            source,
        })?;

        let wanted = session.wanted();
        let mut shots = Vec::new();
        let mut from = 0u64;
        for (i, step) in session.steps.iter().enumerate() {
            let to = recording
                .boundaries
                .get(i)
                .copied()
                .unwrap_or(from + step.duration_ms);
            if step.wanted {
                let cast = work.join(format!("{}.cast", step.key));
                cast::write(
                    &cast,
                    &recording.events,
                    &recording.hidden,
                    recording.cols,
                    recording.rows,
                    from,
                    to,
                )
                .map_err(|source| CaptureError::Io {
                    path: cast.display().to_string(),
                    source,
                })?;

                let clip = out_dir.join(format!("{}.mp4", step.key));
                self.draw(&cast, &clip, session, frame, &step.span)?;
                let _ = std::fs::remove_file(&cast);
                shots.push(Shot {
                    key: step.key,
                    path: clip,
                });
                on_progress(Progress {
                    scene: session.scene.clone(),
                    span: step.span.clone(),
                    done: shots.len(),
                    of: wanted,
                });
            }
            from = to;
        }
        let _ = std::fs::remove_dir_all(&work);
        Ok(shots)
    }
}

impl VhsCapture {
    /// A recording into a clip: `agg` draws the frames, ffmpeg containers
    /// them.
    fn draw(
        &self,
        cast: &Path,
        clip: &Path,
        session: &Session,
        frame: &Frame,
        span: &str,
    ) -> Result<(), CaptureError> {
        let failed = |reason: String| CaptureError::Failed {
            backend: self.id().to_string(),
            span: span.to_string(),
            reason,
        };

        let gif = cast.with_extension("gif");
        let status = Command::new(&self.agg)
            .args(["--theme", session.setting("theme", "asciinema")])
            .args(["--font-size", &session.number("font_size", 18).to_string()])
            // The last frame is what a held gap holds, and a recording
            // whose last frame lasts a single frame time gives the renderer
            // nothing to freeze.
            .args(["--last-frame-duration", "0.2"])
            .arg(cast)
            .arg(&gif)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| failed(format!("{} could not be run: {e}", self.agg)))?;
        if !status.success() {
            return Err(failed(format!("{} exited {status}", self.agg)));
        }

        // Padded to the output frame here rather than left to the renderer
        // so that a clip is a clip: the same file plays on its own and
        // concatenates with every other one without being re-scaled.
        let status = Command::new(&self.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&gif)
            .args([
                "-vf",
                &format!(
                    "scale={w}:{h}:force_original_aspect_ratio=decrease,\
                     pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=0x0b0d10,setsar=1,fps={fps}",
                    w = frame.width,
                    h = frame.height,
                    fps = frame.fps
                ),
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(clip)
            .stdin(Stdio::null())
            .status()
            .map_err(|e| failed(format!("{} could not be run: {e}", self.ffmpeg)))?;
        let _ = std::fs::remove_file(&gif);
        if !status.success() {
            return Err(failed(format!("{} exited {status}", self.ffmpeg)));
        }
        Ok(())
    }
}
