//! Exporting a slides scene with the deck's own Slidev.
//!
//! One `slidev export` per session, of just the slides it needs, gives a
//! still per click step; each wanted shot's still becomes a one-second
//! clip. One second because the renderer fits a clip to its slot — a
//! short one holds its last frame — and a still held is the same picture
//! at any length, which is why the length is not in the key.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use teleprompt_capture::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session};

use crate::scene::{parse, Step};

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
        teleprompt_capture::tool::missing(&["node", &self.ffmpeg])
    }

    fn needs(&self) -> &'static [&'static str] {
        &["node", "ffmpeg", "slidev"]
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
            backend: "slidev".into(),
            shot: shot.into(),
            reason,
        };

        let deck = absolute(Path::new(session.setting("deck", "slides.md")));
        let dir = deck.parent().unwrap_or(Path::new(".")).to_path_buf();
        let Some(slidev) = dir
            .ancestors()
            .map(|d| d.join("node_modules/.bin/slidev"))
            .find(|p| p.is_file())
        else {
            return Err(CaptureError::Unavailable {
                backend: "slidev".into(),
                reason: format!(
                    "no Slidev is installed for {}; run `npm install` beside it",
                    deck.display()
                ),
            });
        };
        let mut steps = Vec::new();
        for shot in &wanted {
            match parse(&shot.source) {
                Ok(Some(step)) => steps.push(step),
                _ => return Err(failed(&shot.id, "the shot names no slide".into())),
            }
        }

        let stills = out_dir.join(format!(".slidev-{}", std::process::id()));
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
            .arg(&stills)
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
        let result = teleprompt_capture::tool::run(&mut export, "slidev export", 6)
            .map(|_| ())
            .map_err(|why| failed(&first, why))
            .and_then(|()| {
                let request = Request {
                    session,
                    frame,
                    out_dir,
                };
                self.encode(&request, &wanted, &steps, &stills, on_progress, &failed)
            });
        let _ = std::fs::remove_dir_all(&stills);
        result
    }
}

/// What [`CaptureBackend::capture`] was asked to record, and where.
struct Request<'a> {
    session: &'a Session,
    frame: &'a Frame,
    out_dir: &'a Path,
}

impl SlidevRender {
    fn encode(
        &self,
        request: &Request<'_>,
        wanted: &[&teleprompt_capture::SessionShot],
        steps: &[Step],
        stills: &Path,
        on_progress: &mut dyn FnMut(Progress),
        failed: &dyn Fn(&str, String) -> CaptureError,
    ) -> Result<Vec<Clip>, CaptureError> {
        let Request {
            session,
            frame,
            out_dir,
        } = *request;
        let mut clips = Vec::new();
        for (shot, step) in wanted.iter().zip(steps) {
            let still = stills.join(still_name(*step));
            if !still.is_file() {
                let has = std::fs::read_dir(stills)
                    .map(|entries| {
                        let prefix = format!("{:03}-", step.slide);
                        entries
                            .flatten()
                            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                            .count()
                    })
                    .unwrap_or(0);
                let why = if has == 0 {
                    format!("the deck has no slide {}", step.slide)
                } else {
                    format!(
                        "slide {} has {} click(s), not {}",
                        step.slide,
                        has - 1,
                        step.clicks
                    )
                };
                return Err(failed(&shot.id, why));
            }
            let clip = out_dir.join(format!("{}.mp4", shot.key));
            let status = Command::new(&self.ffmpeg)
                .args(clip_args(&still, frame, &clip))
                .stdin(Stdio::null())
                .status()
                .map_err(|e| failed(&shot.id, format!("{} could not be run: {e}", self.ffmpeg)))?;
            if !status.success() {
                return Err(failed(&shot.id, format!("{} exited {status}", self.ffmpeg)));
            }
            clips.push(Clip {
                key: shot.key,
                path: clip,
            });
            on_progress(Progress {
                scene: session.scene.clone(),
                shot: shot.id.clone(),
                done: clips.len(),
                of: wanted.len(),
            });
        }
        Ok(clips)
    }
}

/// `path` made absolute, since the export runs in another directory.
fn absolute(path: &Path) -> PathBuf {
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}
