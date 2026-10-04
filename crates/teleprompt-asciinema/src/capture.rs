//! Rendering a recording scene with `agg`, asciinema's own renderer.
//!
//! A session's shots are laid end to end at their scheduled offsets — each
//! shot's events, then its screen held until its slot is over — into one
//! cast, which `agg` renders once. A terminal shot opens on the screen the
//! one before it left, so the session is replayed from its start; the reel
//! is then cut into a clip per wanted shot.

use std::path::Path;
use std::process::Command;

use teleprompt_plugin::capture::reel::{cut, starved, windows};
use teleprompt_plugin::capture::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session};

use crate::scene::{parse, write, Cast, Event};

/// An escape that changes nothing on screen: it gives `agg` a frame at a
/// time where the recording has none, so a shot that is all held screen
/// still has footage.
const NOTHING: &str = "\u{1b}[0m";

/// The session as one cast, each shot starting where the schedule put it.
pub fn session_cast(session: &Session) -> Result<Cast, (String, String)> {
    let mut all: Option<Cast> = None;
    for (shot, (from_ms, duration_ms)) in session.shots.iter().zip(windows(session)) {
        let part = parse(&shot.source)
            .map_err(|_| (shot.id.to_string(), "the shot is not a cast".to_string()))?;
        let from = from_ms as f64 / 1000.0;
        let cast = all.get_or_insert_with(|| Cast {
            events: Vec::new(),
            duration: 0.0,
            ..part.clone()
        });
        cast.events.push(Event {
            time: from,
            code: "o".into(),
            data: NOTHING.into(),
        });
        cast.events.extend(
            part.events
                .into_iter()
                .filter(|e| e.time * 1000.0 < duration_ms as f64)
                .map(|e| Event {
                    time: from + e.time,
                    ..e
                }),
        );
        cast.duration = from + duration_ms as f64 / 1000.0;
    }
    let mut cast = all.ok_or_else(|| (String::new(), "the session has no shots".to_string()))?;
    cast.events.push(Event {
        time: cast.duration,
        code: "o".into(),
        data: NOTHING.into(),
    });
    Ok(cast)
}

/// Records `asciinema` scenes by rendering the recording with `agg`.
#[derive(Debug, Clone)]
pub struct AsciinemaRender {
    pub agg: String,
    pub ffmpeg: String,
}

impl Default for AsciinemaRender {
    fn default() -> Self {
        Self {
            agg: "agg".into(),
            ffmpeg: "ffmpeg".into(),
        }
    }
}

impl CaptureBackend for AsciinemaRender {
    fn unavailable(&self) -> Option<String> {
        teleprompt_plugin::tool::missing(&[&self.agg, &self.ffmpeg])
    }

    fn needs(&self) -> &'static [&'static teleprompt_plugin::tool::Tool] {
        static NEEDS: &[&teleprompt_plugin::tool::Tool] =
            &[&crate::tools::AGG, &teleprompt_plugin::tool::FFMPEG];
        NEEDS
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
            .map(|s| s.id.to_string())
            .unwrap_or_default();
        let failed = |shot: &str, reason: String| CaptureError::Failed {
            backend: "asciinema".into(),
            shot: shot.into(),
            reason,
        };
        let cast = session_cast(session).map_err(|(s, why)| failed(&s, why))?;

        let work = out_dir.join(format!(".asciinema-{}", std::process::id()));
        let request = Request {
            session,
            frame,
            out_dir,
        };
        let result = self.render(&request, &cast, &work, on_progress, &failed, &first);
        let _ = std::fs::remove_dir_all(&work);
        result
    }
}

/// What [`CaptureBackend::capture`] was asked to record, and where.
struct Request<'a> {
    session: &'a Session,
    frame: &'a Frame,
    out_dir: &'a Path,
}

impl AsciinemaRender {
    fn render(
        &self,
        request: &Request<'_>,
        cast: &Cast,
        work: &Path,
        on_progress: &mut dyn FnMut(Progress),
        failed: &dyn Fn(&str, String) -> CaptureError,
        first: &str,
    ) -> Result<Vec<Clip>, CaptureError> {
        let Request {
            session,
            frame,
            out_dir,
        } = *request;
        let io = |e: std::io::Error| failed(first, format!("{}: {e}", work.display()));
        std::fs::create_dir_all(work).map_err(io)?;
        let (input, gif, reel) = (
            work.join("session.cast"),
            work.join("session.gif"),
            work.join("session.mp4"),
        );
        std::fs::write(&input, write(cast)).map_err(io)?;

        // The cast is already paced by the schedule: no idle limit, no
        // loop, no extra hold at the end.
        let mut agg = Command::new(&self.agg);
        agg.arg("--idle-time-limit")
            .arg("100000")
            .arg("--no-loop")
            .arg("--last-frame-duration")
            .arg("0")
            .arg("--fps-cap")
            .arg(frame.fps.to_string())
            .arg("--font-size")
            .arg(session.setting("font_size", "32"));
        if let Some(theme) = session.settings.get("theme") {
            agg.arg("--theme").arg(theme);
        }
        self.run(agg.arg(&input).arg(&gif), first, failed)?;
        // One conversion to a codec the cuts can seek in, at even sizes
        // for yuv420p; scaling to the frame is the renderer's job.
        self.run(
            Command::new(&self.ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
                .arg(&gif)
                .args([
                    "-vf",
                    "scale=trunc(iw/2)*2:trunc(ih/2)*2",
                    "-pix_fmt",
                    "yuv420p",
                ])
                .arg(&reel),
            first,
            failed,
        )?;
        if let Some(why) = starved(&reel, session) {
            return Err(failed(first, why));
        }

        let mut clips = Vec::new();
        for (shot, (from_ms, duration_ms)) in session.shots.iter().zip(windows(session)) {
            if !shot.wanted {
                continue;
            }
            let clip = out_dir.join(format!("{}.mp4", shot.key));
            cut(&self.ffmpeg, &reel, &clip, from_ms, duration_ms)
                .map_err(|why| failed(&shot.id, why))?;
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
        Ok(clips)
    }

    fn run(
        &self,
        command: &mut Command,
        first: &str,
        failed: &dyn Fn(&str, String) -> CaptureError,
    ) -> Result<(), CaptureError> {
        let program = command.get_program().to_string_lossy().to_string();
        teleprompt_plugin::tool::run(command, &program, 4)
            .map(|_| ())
            .map_err(|why| failed(first, why))
    }
}
