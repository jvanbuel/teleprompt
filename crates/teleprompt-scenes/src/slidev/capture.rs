//! Exporting a slides scene with the deck's own Slidev.
//!
//! One `slidev export` per session, of just the slides it needs, gives a
//! still per click step; each wanted shot's still becomes a one-second
//! clip. One second because the renderer fits a clip to its slot — a
//! short one holds its last frame — and a still held is the same picture
//! at any length, which is why the length is not in the key.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

use teleprompt_scene::capture::{
    absolute, CaptureBackend, CaptureError, Clip, Frame, Job, Progress, Session,
};

use crate::slidev::scene::{parse, Step};

/// The still `slidev export --with-clicks` writes for a step: slide `003`
/// with no clicks is `003-01.png`, after one click `003-02.png`.
pub fn still_name(step: Step) -> String {
    format!("{:03}-{:02}.png", step.slide, step.clicks + 1)
}

/// `--range`: every slide a session's wanted shots show, once each.
pub fn range(steps: &[Step]) -> String {
    let slides: BTreeSet<u32> = steps.iter().map(|s| s.slide).collect();
    slides
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// The ffmpeg arguments that make one still into a clip filling `frame`,
/// letterboxed rather than stretched.
pub(crate) fn clip_args(still: &Path, frame: &Frame, clip: &Path) -> Vec<String> {
    let (w, h) = (frame.width, frame.height);
    vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-loop".into(),
        "1".into(),
        "-framerate".into(),
        frame.fps.to_string(),
        "-i".into(),
        still.display().to_string(),
        "-t".into(),
        "1".into(),
        "-vf".into(),
        format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,format=yuv420p"
        ),
        "-c:v".into(),
        "libx264".into(),
        clip.display().to_string(),
    ]
}

/// Records `slidev` scenes by exporting the deck's slides.
#[derive(Debug, Clone)]
pub struct SlidevRender {
    pub ffmpeg: String,
}

impl Default for SlidevRender {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
        }
    }
}

impl CaptureBackend for SlidevRender {
    fn unavailable(&self) -> Option<String> {
        // The deck's `slidev` is a Node script.
        teleprompt_scene::core::tool::missing(&["node", &self.ffmpeg])
    }

    fn needs(&self) -> &'static [&'static teleprompt_scene::core::tool::Tool] {
        static NEEDS: &[&teleprompt_scene::core::tool::Tool] = &[
            &teleprompt_scene::core::tool::NODE,
            &teleprompt_scene::core::tool::FFMPEG,
            &crate::slidev::tools::SLIDEV,
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
        let mut job = Job::new("slidev", session, frame, out_dir, on_progress);
        let deck = absolute(&session.path("deck", "slides.md"));
        let dir = deck.parent().unwrap_or(Path::new(".")).to_path_buf();
        let Some(slidev) = dir
            .ancestors()
            .map(|d| d.join("node_modules/.bin/slidev"))
            .find(|p| p.is_file())
        else {
            return Err(job.unavailable(format!(
                "no Slidev is installed for {}; run `npm install` beside it",
                deck.display()
            )));
        };
        let mut steps = Vec::new();
        for shot in job.wanted() {
            match parse(&shot.source) {
                Ok(Some(step)) => steps.push(step),
                _ => return Err(job.failed(&shot.id, "the shot names no slide")),
            }
        }

        let stills = job.work_dir()?;
        let mut export = Command::new(&slidev);
        export
            .arg("export")
            .arg(&deck)
            .args([
                "--format",
                "png",
                "--with-clicks",
                "--range",
                &range(&steps),
            ])
            .arg("--output")
            .arg(&*stills)
            .current_dir(&dir)
            .stdin(Stdio::null());
        if session.setting("dark", "false") == "true" {
            export.arg("--dark");
        }
        // The scene's `browser`, else the machine's; neither means
        // Slidev's own Playwright finds one.
        if let Some(browser) = session
            .settings
            .get("browser")
            .cloned()
            .or_else(|| std::env::var("TELEPROMPT_SLIDEV_BROWSER").ok())
        {
            export.arg("--executable-path").arg(browser);
        }
        teleprompt_scene::core::tool::run(&mut export, "slidev export", 6)
            .map_err(|why| job.failed_all(why))?;

        for (shot, step) in job.wanted().zip(steps) {
            let still = stills.join(still_name(step));
            if !still.is_file() {
                return Err(job.failed(&shot.id, missing_still(&stills, step)));
            }
            let clip = job.clip_path(shot);
            teleprompt_scene::core::tool::run(
                Command::new(&self.ffmpeg).args(clip_args(&still, frame, &clip)),
                &self.ffmpeg,
                1,
            )
            .map_err(|why| job.failed(&shot.id, why))?;
            job.keep(shot);
        }
        Ok(job.clips())
    }
}

/// Why the export left no still for `step`: no such slide, or not that
/// many clicks on it.
fn missing_still(stills: &Path, step: Step) -> String {
    let has = std::fs::read_dir(stills)
        .map(|entries| {
            let prefix = format!("{:03}-", step.slide);
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                .count()
        })
        .unwrap_or(0);
    if has == 0 {
        format!("the deck has no slide {}", step.slide)
    } else {
        format!(
            "slide {} has {} click(s), not {}",
            step.slide,
            has - 1,
            step.clicks
        )
    }
}
