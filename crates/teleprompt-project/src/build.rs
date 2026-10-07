//! `build`: the whole pipeline, ending in a file you can play. It renders
//! from the manifest `dub` just published, as an outside consumer would
//! (docs/design.md#rendering).

use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_manifest::chapters;
use teleprompt_render::incremental::IncrementalRenderer;
use teleprompt_render::plan::{self, Inputs};
/// A render's progress, as [`Builder::build`] reports it.
pub use teleprompt_render::Progress;
use teleprompt_render::{Picture, RenderError};
use teleprompt_scene::capture::Frame;
use teleprompt_scene::ScenePlugins;

use crate::cache;
use crate::capture::Scenes;
use crate::dub::{captured_shot, Dubber};
use crate::project::{CacheDir, Clips, Compose, Script};
use crate::Failure;
use teleprompt_core::Reporter;

/// A frame size and rate that override a script's `output:` frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameOverride {
    pub resolution: Option<(u32, u32)>,
    pub fps: Option<u32>,
}

impl FrameOverride {
    /// The frame to record and render at: the script's `output:` frame,
    /// with these overrides.
    pub fn frame(&self, output: &teleprompt_core::config::OutputConfig) -> Frame {
        let (width, height) = self.resolution.unwrap_or(output.resolution);
        Frame {
            width,
            height,
            fps: self.fps.unwrap_or(output.fps),
        }
    }
}

/// Builds a script's video: dubs it, records what its shots are missing,
/// and renders from the manifest it just published. Every setting starts
/// at what an author gets with no flags.
#[derive(Clone)]
pub struct Builder<'s> {
    script: &'s Script,
    out: PathBuf,
    narration_root: PathBuf,
    clips_dir: CacheDir<Clips>,
    frame: FrameOverride,
    compose_dir: Option<CacheDir<Compose>>,
    cache_max_mb: u64,
    plugins: &'s ScenePlugins,
}

impl<'s> Builder<'s> {
    pub fn new(script: &'s Script) -> Self {
        let project = script.project();
        let stem = script
            .path()
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "video".into());
        Self {
            script,
            // `build/` is what `new` gitignores. The name follows the
            // committed timelines': `cli.en.json` beside `cli.en.mp4`.
            out: project
                .root
                .join("build")
                .join(format!("{stem}.{}.mp4", script.locale())),
            // A real directory, not a temporary one, so the render can be
            // checked against its manifest.
            narration_root: project.root.join("build").join("narration"),
            clips_dir: project.caches().clips(),
            frame: FrameOverride::default(),
            compose_dir: Some(project.caches().compose()),
            cache_max_mb: cache::DEFAULT_MAX_MB,
            plugins: project.registry.scenes,
        }
    }

    /// The video to write.
    pub fn out(self, out: PathBuf) -> Self {
        Self { out, ..self }
    }

    /// Where the video will be written.
    pub fn output(&self) -> &Path {
        &self.out
    }

    /// The frame size and rate, over the script's `output:` frame.
    pub fn frame(self, frame: FrameOverride) -> Self {
        Self { frame, ..self }
    }

    pub fn resolution(mut self, width: u32, height: u32) -> Self {
        self.frame.resolution = Some((width, height));
        self
    }

    pub fn fps(mut self, fps: u32) -> Self {
        self.frame.fps = Some(fps);
        self
    }

    /// Re-encodes the whole video rather than reuse the compose cache
    /// (docs/design.md#compose-cache).
    pub fn no_cache(self) -> Self {
        Self {
            compose_dir: None,
            ..self
        }
    }

    /// Megabytes of encoded video to keep afterwards, least recently used
    /// evicted first. Every build prunes, so the cap holds without anyone
    /// remembering to run a command.
    pub fn cache_max_mb(self, cache_max_mb: u64) -> Self {
        Self {
            cache_max_mb,
            ..self
        }
    }

    /// The scene plugins that record its shots, so a test can build on a
    /// machine that cannot record a scene, and check the warning and slate
    /// that follow.
    pub fn plugins(self, plugins: &'s ScenePlugins) -> Self {
        Self { plugins, ..self }
    }

    /// `--no-cache` switches off reuse, not the renderer, so both encode
    /// through one path.
    fn renderer(&self) -> IncrementalRenderer {
        IncrementalRenderer {
            program: "ffmpeg".into(),
            cache_dir: self
                .compose_dir
                .as_ref()
                .map_or_else(std::env::temp_dir, |dir| dir.to_path_buf()),
            reuse: self.compose_dir.is_some(),
        }
    }

    /// Dubs the script, then records whatever its shots are missing from
    /// the clip directory: `build` short of rendering. Dubbed afresh rather
    /// than read from disk, where the manifest may be stale
    /// (docs/design.md#rendering).
    pub async fn capture(
        &self,
        reporter: &dyn Reporter,
    ) -> Result<crate::capture::CaptureReport, Failure> {
        let dubbed = Dubber::new(self.script)
            .reporting(reporter)
            .dub(&self.narration_root)
            .await?;
        let frame = self.frame.frame(&dubbed.output);
        Ok(Scenes::new(self.plugins, &self.clips_dir, frame)
            .capture(&dubbed, &mut |p| reporter.progress(captured_shot(p))))
    }

    /// Dubs, captures, then renders what was dubbed, as an outside
    /// consumer would (docs/design.md#rendering).
    pub async fn build(&self, reporter: &dyn Reporter) -> Result<BuildReport, Failure> {
        let locale = self.script.locale();
        let dubbed = Dubber::new(self.script)
            .reporting(reporter)
            .dub(&self.narration_root)
            .await?;
        // Stage 5 (docs/design.md#pipeline). What cannot be recorded becomes
        // a warning and a slate, not a failure; see [`Scenes::capture`].
        let frame = self.frame.frame(&dubbed.output);
        let recorded = Scenes::new(self.plugins, &self.clips_dir, frame)
            .capture(&dubbed, &mut |p| reporter.progress(captured_shot(p)));
        let mut warnings = dubbed.warnings;
        warnings.extend(recorded.warnings);

        let narration = self.narration_root.join(locale);
        let (render_plan, mut plan_warnings) = plan::from_manifest(
            &dubbed.manifest,
            &Inputs {
                narration_dir: narration.clone(),
                clips_dir: self.clips_dir.to_path_buf(),
                output: self.out.clone(),
                width: frame.width,
                height: frame.height,
                fps: frame.fps,
                names: dubbed.output.names,
            },
        );
        let slates = render_plan
            .shots
            .iter()
            .filter(|b| b.picture == Picture::Slate)
            .count();

        let renderer = self.renderer();
        let rendered = renderer
            .render(&render_plan, &mut |p: Progress| {
                reporter.progress(teleprompt_core::Progress::Render {
                    done_ms: p.rendered_ms,
                    of_ms: p.of_ms,
                });
            })
            .map_err(|e| Failure::Runtime(explain(&e)))?;

        // Pruned after the render, never before: pruning first could evict
        // pieces this render was about to copy.
        warnings.append(&mut plan_warnings);
        warnings.extend(self.prune_compose_cache());

        let [srt, vtt, chapters] = copy_beside(&rendered.path, &narration)?;
        if dubbed.manifest.chapters.len() > 1 {
            warnings.extend(chapters::youtube(&dubbed.manifest).1);
        }

        Ok(BuildReport {
            output: rendered.path,
            captions: vec![srt, vtt],
            chapters,
            duration_ms: rendered.duration_ms,
            renderer: renderer.id(),
            lines: dubbed.manifest.lines.len(),
            items: dubbed.manifest.shots.len(),
            slates,
            captured: recorded.captured,
            reused_ms: rendered.reused_ms,
            warnings,
        })
    }

    /// Shrinks the compose cache to `cache_max_mb`; what went wrong, if it
    /// did. A cache that will not shrink is a disk problem, not a reason to
    /// report a video that exists as a failure.
    fn prune_compose_cache(&self) -> Option<String> {
        let dir = self.compose_dir.as_ref()?;
        let limit = self.cache_max_mb * 1_048_576;
        cache::prune(dir, limit).err().map(|e| {
            format!(
                "could not prune the compose cache at {}: {e}",
                dir.display()
            )
        })
    }
}

#[derive(Debug, Serialize)]
pub struct BuildReport {
    pub output: PathBuf,
    /// Subtitles beside the video, as SubRip and WebVTT.
    pub captions: Vec<PathBuf>,
    /// The chapters beside the video, as a YouTube description lists them.
    pub chapters: PathBuf,
    pub duration_ms: u64,
    /// Which renderer did the work.
    pub renderer: &'static str,
    pub lines: usize,
    pub items: usize,
    /// Shots that rendered as a slate because nothing had captured them.
    pub slates: usize,
    /// Shots recorded on the way past. A warm project records none and
    /// renders the same video.
    pub captured: usize,
    /// How much of the picture was copied from the compose cache instead
    /// of encoded. `null` from a render that had no cache to draw on.
    pub reused_ms: Option<u64>,
    pub warnings: Vec<String>,
}

impl BuildReport {
    /// The human report. Warnings are not in it: `main` prints them to
    /// stderr.
    pub fn render(&self) -> String {
        let reused = match self.reused_ms {
            Some(ms) if ms > 0 => format!(", {} reused", clock(ms)),
            _ => String::new(),
        };
        format!(
            "  {}\n  {} via {}{reused}\n  {} line(s), {} item(s), {} slate(s)\n  \
             captions and chapters beside it: {}, chapters.txt\n",
            self.output.display(),
            clock(self.duration_ms),
            self.renderer,
            self.lines,
            self.items,
            self.slates,
            self.captions
                .iter()
                .filter_map(|p| p.extension()?.to_str())
                .collect::<Vec<_>>()
                .join(", "),
        )
    }
}

/// `m:ss.mmm`, the way a player would show it.
fn clock(ms: u64) -> String {
    format!("{}:{:02}.{:03}", ms / 60_000, (ms / 1000) % 60, ms % 1000)
}

/// The subtitles and chapters `dub` wrote, copied beside the video and
/// named after it (`tour.en.srt` beside `tour.en.mp4`), where players look
/// for them.
fn copy_beside(video: &Path, narration: &Path) -> Result<[PathBuf; 3], Failure> {
    let copy = |name: &str, ext: &str| {
        let (from, path) = (narration.join(name), video.with_extension(ext));
        std::fs::copy(&from, &path)
            .map(|_| path.clone())
            .map_err(|e| {
                Failure::Runtime(format!(
                    "cannot copy {} to {}: {e}",
                    from.display(),
                    path.display()
                ))
            })
    };
    Ok([
        copy("captions.srt", "srt")?,
        copy("captions.vtt", "vtt")?,
        copy("chapters.txt", "chapters.txt")?,
    ])
}

/// A render failure, with the one thing the author can act on in front.
fn explain(error: &RenderError) -> String {
    match error {
        RenderError::Unavailable { program, .. } => format!(
            "{program} is not on PATH. Rendering needs it; `teleprompt setup {program}` \
             says how to install it."
        ),
        other => other.to_string(),
    }
}

/// `WIDTHxHEIGHT`, as `--resolution` takes it.
///
/// Rejects rather than guesses: a typo silently rendered at some default
/// is a long wait for the wrong file.
pub fn parse_resolution(text: &str) -> Result<(u32, u32), String> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("`{text}` is not a resolution (expected e.g. `1920x1080`)"))?;
    let parse = |part: &str, which: &str| {
        part.trim()
            .parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("`{text}` has no usable {which} (expected e.g. `1920x1080`)"))
    };
    Ok((parse(w, "width")?, parse(h, "height")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resolution_is_two_numbers() {
        assert_eq!(parse_resolution("1920x1080"), Ok((1920, 1080)));
        assert_eq!(parse_resolution("1280X720"), Ok((1280, 720)));
    }

    #[test]
    fn anything_else_is_reported_rather_than_guessed_at() {
        for bad in ["1080p", "1920", "1920x", "0x1080", "-16x9", "1920x1080x30"] {
            assert!(
                parse_resolution(bad).is_err(),
                "`{bad}` was accepted as a resolution"
            );
        }
    }
}
