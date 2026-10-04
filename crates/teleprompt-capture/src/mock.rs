//! The reference capture backend: a flat field of colour per shot.
//!
//! The counterpart of the `mock` scene adapter. A mock shot says nothing
//! about what is on screen, so its picture is a colour taken from its
//! capture key: two shots look different exactly when they are different.
//! It lets the rest of the stage be tested without a terminal or display.

use std::path::Path;
use std::process::{Command, Stdio};

use teleprompt_core::Hash;

use crate::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session};

#[derive(Debug, Clone)]
pub struct MockCapture {
    /// The binary that draws the frames: a flat field is one `color` source.
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
    fn unavailable(&self) -> Option<String> {
        crate::tool::missing(&[&self.program])
    }

    fn needs(&self) -> &'static [&'static str] {
        &["ffmpeg"]
    }

    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Clip>, CaptureError> {
        std::fs::create_dir_all(out_dir).map_err(|source| CaptureError::Io {
            path: out_dir.display().to_string(),
            source,
        })?;

        let wanted = session.wanted();
        let mut clips = Vec::new();
        for shot in &session.shots {
            // Nothing to keep. A real backend still replays this shot.
            if !shot.wanted {
                continue;
            }
            let path = out_dir.join(format!("{}.mp4", shot.key));
            let status = Command::new(&self.program)
                .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
                .arg("-t")
                .arg(teleprompt_core::time::ffmpeg_seconds(shot.duration_ms))
                .arg("-i")
                .arg(format!(
                    "color=c={}:s={}x{}:r={}",
                    colour_of(&shot.key),
                    frame.width,
                    frame.height,
                    frame.fps
                ))
                .args(["-pix_fmt", "yuv420p"])
                .arg(&path)
                .stdin(Stdio::null())
                .status()
                .map_err(|e| CaptureError::Unavailable {
                    backend: "mock".to_string(),
                    reason: format!("{} could not be run: {e}", self.program),
                })?;
            if !status.success() {
                return Err(CaptureError::Failed {
                    backend: "mock".to_string(),
                    shot: shot.id.clone(),
                    reason: format!("{} exited {status}", self.program),
                });
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
        Ok(clips)
    }
}
