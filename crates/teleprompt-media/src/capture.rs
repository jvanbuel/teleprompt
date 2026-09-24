//! Turning media directives into clips with ffmpeg.
//!
//! Each wanted shot is one ffmpeg run. A still — an image, a title — is a
//! one-second clip the renderer holds for its sentence; a clip is its
//! range, or its slot's length from `from` when it names no `to`. A clip's
//! own sound is dropped: the narration is the soundtrack.

use std::path::Path;
use std::process::Command;

use teleprompt_capture::{
    CaptureBackend, CaptureError, Clip, Frame, Progress, Session, SessionShot,
};

use crate::scene::{directive, Directive};

/// A value inside an ffmpeg filter argument.
fn filter_value(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

/// Fit a picture to the frame: letterboxed, or filling it and cropped.
fn fit(frame: &Frame, cover: bool, background: &str) -> String {
    let (w, h) = (frame.width, frame.height);
    if cover {
        format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}")
    } else {
        format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color={}",
            filter_value(background)
        )
    }
}

/// The ffmpeg arguments for one shot. `dir` is the media directory and
/// `work` where a title's text files are written.
pub fn args(
    shot: &SessionShot,
    session: &Session,
    frame: &Frame,
    dir: &Path,
    work: &Path,
    clip: &Path,
) -> Option<Vec<String>> {
    let background = session.setting("background", "#0b0d10");
    let color = session.setting("color", "#eef3f8");
    let font = session.setting("font", "Sans");
    let fps = frame.fps.to_string();
    let seconds = teleprompt_core::time::ffmpeg_seconds;
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y"]
        .map(String::from)
        .to_vec();
    let vf = match directive(&shot.source)? {
        Directive::Image { src, cover } => {
            a.extend(["-loop", "1", "-framerate", &fps, "-t", "1", "-i"].map(String::from));
            a.push(dir.join(src).display().to_string());
            fit(frame, cover, background)
        }
        Directive::Clip {
            src,
            from_ms,
            to_ms,
            cover,
        } => {
            let length = to_ms.map_or(shot.duration_ms, |to| to - from_ms);
            a.extend([
                "-ss".into(),
                seconds(from_ms),
                "-t".into(),
                seconds(length),
                "-i".into(),
            ]);
            a.push(dir.join(src).display().to_string());
            a.push("-an".into());
            format!("{},fps={fps}", fit(frame, cover, background))
        }
        Directive::Title { text, subtitle } => {
            a.extend(["-f", "lavfi", "-t", "1", "-i"].map(String::from));
            a.push(format!(
                "color=c={}:s={}x{}:r={fps}",
                filter_value(background),
                frame.width,
                frame.height
            ));
            // The text goes through files, with expansion off, so nothing
            // an author writes is read as filter syntax or a `%{…}`.
            let key = shot.key.to_string();
            let draw = |file: &str, size: u32, y: &str| {
                format!(
                    "drawtext=textfile='{}':expansion=none:font='{}':fontcolor={}:\
                     fontsize={size}:x=(w-text_w)/2:y={y}",
                    filter_value(&work.join(file).display().to_string()),
                    filter_value(font),
                    filter_value(color),
                )
            };
            let _ = std::fs::write(work.join(format!("{key}.title")), &text);
            let title_size = frame.height / 11;
            match subtitle {
                Some(sub) => {
                    let _ = std::fs::write(work.join(format!("{key}.subtitle")), sub);
                    format!(
                        "{},{}",
                        draw(&format!("{key}.title"), title_size, "h/2-text_h"),
                        draw(&format!("{key}.subtitle"), frame.height / 24, "h/2+h/30")
                    )
                }
                None => draw(&format!("{key}.title"), title_size, "(h-text_h)/2"),
            }
        }
    };
    a.extend(["-vf".into(), format!("{vf},format=yuv420p")]);
    a.extend(["-c:v", "libx264", "-r"].map(String::from));
    a.push(fps);
    a.push(clip.display().to_string());
    Some(a)
}

/// Records `media` scenes with ffmpeg.
#[derive(Debug, Clone)]
pub struct MediaRender {
    pub ffmpeg: String,
}

impl Default for MediaRender {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
        }
    }
}

impl CaptureBackend for MediaRender {
    fn id(&self) -> &'static str {
        "media"
    }

    fn adapter(&self) -> &'static str {
        "media"
    }

    fn unavailable(&self) -> Option<String> {
        teleprompt_capture::tool::missing(&[&self.ffmpeg])
    }

    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Clip>, CaptureError> {
        let failed = |shot: &str, reason: String| CaptureError::Failed {
            backend: "media".into(),
            shot: shot.to_string(),
            reason,
        };
        let dir = Path::new(session.setting("dir", "media"));
        let work = out_dir.join(format!(".media-{}", std::process::id()));
        std::fs::create_dir_all(&work)
            .map_err(|e| failed("", format!("{}: {e}", work.display())))?;

        let mut clips = Vec::new();
        let mut result = Ok(());
        for shot in session.shots.iter().filter(|s| s.wanted) {
            let clip = out_dir.join(format!("{}.mp4", shot.key));
            let Some(a) = args(shot, session, frame, dir, &work, &clip) else {
                result = Err(failed(&shot.id, "the shot is not a media directive".into()));
                break;
            };
            if let Err(why) =
                teleprompt_capture::tool::run(Command::new(&self.ffmpeg).args(&a), &self.ffmpeg, 1)
            {
                result = Err(failed(&shot.id, why));
                break;
            }
            clips.push(Clip {
                key: shot.key,
                path: clip,
            });
            on_progress(Progress {
                scene: session.scene.clone(),
                shot: shot.id.clone(),
                done: clips.len(),
                of: session.wanted(),
            });
        }
        let _ = std::fs::remove_dir_all(&work);
        result.map(|()| clips)
    }
}
