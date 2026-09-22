//! The reference capture backend: a flat field of colour per step.
//!
//! The counterpart of [`teleprompt_scene::mock`] at the other end of the
//! pipeline, and it exists for the same reason. A mock scene's source says
//! how long things take and nothing about what is on screen, so the honest
//! picture for one is a field that is *identifiably this step* and claims
//! nothing else: the colour is taken from the step's capture key, so two
//! steps look different exactly when they are different.
//!
//! It is also what makes the rest of the stage testable. Everything a real
//! backend has to get right — running a session in order, keeping only the
//! wanted steps, filing a clip under its key, fitting the slot — is
//! exercised here without a terminal, a display or a font.

use std::path::Path;
use std::process::{Command, Stdio};

use teleprompt_core::Hash;

use crate::{CaptureBackend, CaptureError, Frame, Progress, Session, Shot};

#[derive(Debug, Clone)]
pub struct MockCapture {
    /// The binary that draws the frames. ffmpeg, because a flat field at a
    /// given size and rate is one `color` source and there is no reason to
    /// link an encoder to produce one.
    pub program: String,
}

impl Default for MockCapture {
    fn default() -> Self {
        Self {
            program: "ffmpeg".into(),
        }
    }
}

/// A colour for a key: readable, distinct, and never black — a clip that
/// renders black is indistinguishable from the slate it replaced.
fn colour_of(key: &Hash) -> String {
    let hex = key.to_string();
    let channel = |i: usize| {
        let byte = u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0x80);
        // Into the top half of the range, so every clip is visibly a clip.
        0x60 + (u16::from(byte) * 0x9f / 0xff) as u8
    };
    format!("0x{:02x}{:02x}{:02x}", channel(0), channel(2), channel(4))
}

impl CaptureBackend for MockCapture {
    fn id(&self) -> &'static str {
        "mock"
    }

    fn adapter(&self) -> &'static str {
        "mock"
    }

    fn unavailable(&self) -> Option<String> {
        let found = Command::new(&self.program)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok();
        (!found).then(|| format!("{} is not on PATH", self.program))
    }

    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Shot>, CaptureError> {
        std::fs::create_dir_all(out_dir).map_err(|source| CaptureError::Io {
            path: out_dir.display().to_string(),
            source,
        })?;

        let wanted = session.wanted();
        let mut shots = Vec::new();
        for step in &session.steps {
            // A step nothing wants is still a step the session ran
            // through; there is simply nothing to keep. A real backend
            // replays it to reach the screen the next one opens on, and
            // this one keeps the shape so that difference stays visible.
            if !step.wanted {
                continue;
            }
            let path = out_dir.join(format!("{}.mp4", step.key));
            let status = Command::new(&self.program)
                .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
                .arg("-t")
                .arg(format!(
                    "{}.{:03}",
                    step.duration_ms / 1000,
                    step.duration_ms % 1000
                ))
                .arg("-i")
                .arg(format!(
                    "color=c={}:s={}x{}:r={}",
                    colour_of(&step.key),
                    frame.width,
                    frame.height,
                    frame.fps
                ))
                .args(["-pix_fmt", "yuv420p"])
                .arg(&path)
                .stdin(Stdio::null())
                .status()
                .map_err(|e| CaptureError::Unavailable {
                    backend: self.id().to_string(),
                    reason: format!("{} could not be run: {e}", self.program),
                })?;
            if !status.success() {
                return Err(CaptureError::Failed {
                    backend: self.id().to_string(),
                    span: step.span.clone(),
                    reason: format!("{} exited {status}", self.program),
                });
            }

            shots.push(Shot {
                key: step.key,
                path,
            });
            on_progress(Progress {
                scene: session.scene.clone(),
                span: step.span.clone(),
                done: shots.len(),
                of: wanted,
            });
        }
        Ok(shots)
    }
}
