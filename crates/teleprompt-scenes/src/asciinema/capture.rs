//! Rendering a recording scene with `agg`, asciinema's own renderer.
//!
//! A session's shots are laid end to end at their scheduled offsets — each
//! shot's events, then its screen held until its slot is over — into one
//! cast, which `agg` renders once. A terminal shot opens on the screen the
//! one before it left, so the session is replayed from its start; the reel
//! is then cut into a clip per wanted shot.

use std::path::Path;
use std::process::Command;

use teleprompt_scene::capture::reel::windows;
use teleprompt_scene::capture::{
    CaptureBackend, CaptureError, Clip, Frame, Job, Progress, Session,
};

use crate::asciinema::scene::{parse, write, Cast, Event};

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
        teleprompt_scene::core::tool::missing(&[&self.agg, &self.ffmpeg])
    }

    fn needs(&self) -> &'static [&'static teleprompt_scene::core::tool::Tool] {
        static NEEDS: &[&teleprompt_scene::core::tool::Tool] = &[
            &crate::asciinema::tools::AGG,
            &teleprompt_scene::core::tool::FFMPEG,
        ];
        NEEDS
    }

    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Clip>, CaptureError> {
        let mut job = Job::new("asciinema", session, frame, out_dir, on_progress);
        let cast = session_cast(session).map_err(|(shot, why)| job.failed(&shot, why))?;
        let work = job.work_dir()?;
        let (input, gif, reel) = (
            work.join("session.cast"),
            work.join("session.gif"),
            work.join("session.mp4"),
        );
        std::fs::write(&input, write(&cast))
            .map_err(|e| job.failed_all(format!("{}: {e}", input.display())))?;

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
        run(&job, agg.arg(&input).arg(&gif))?;
        // One conversion to a codec the cuts can seek in, at even sizes
        // for yuv420p; scaling to the frame is the renderer's job.
        run(
            &job,
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
        )?;
        job.cut_reel(&self.ffmpeg, &reel)?;
        Ok(job.clips())
    }
}

/// `command` run to the end; the session fails with what it said if not.
fn run(job: &Job<'_>, command: &mut Command) -> Result<(), CaptureError> {
    let program = command.get_program().to_string_lossy().to_string();
    teleprompt_scene::core::tool::run(command, &program, 4)
        .map(|_| ())
        .map_err(|why| job.failed_all(why))
}
