//! `build`: the whole pipeline, ending in a file you can play.
//!
//! Everything before this command decided *when* each thing happens.
//! `build` turns those decisions into a video, and it does it by reading
//! the published narration manifest — the same artifact `serve` reads and
//! the same one an outside integrator reads. It could have kept the
//! in-process `Timeline` in hand instead; two timing paths drift, and the
//! one that drifts silently is the one nobody renders from.

use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_render::ffmpeg::FfmpegRenderer;
use teleprompt_render::plan::{self, Inputs};
use teleprompt_render::{Picture, Progress, RenderError, Renderer};

use crate::cmd::check::cache_root;
use crate::cmd::dub::{self, DubError};
use crate::project::Project;

/// Everything `build` needs beyond the script itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    /// The video to write.
    pub out: PathBuf,
    /// Where narration is published on the way past. A real directory
    /// rather than a temporary one: it is the manifest `serve` and any
    /// integrator read, and a render that cannot be traced back to a
    /// manifest cannot be checked against one.
    pub narration_root: PathBuf,
    /// Where captured clips are looked up, by span hash.
    pub clips_dir: PathBuf,
    /// A frame size from the command line, which overrides the script's
    /// own `output.resolution`. `None` — the usual case — leaves the
    /// script in charge of how it looks.
    pub resolution: Option<(u32, u32)>,
    /// A frame rate from the command line, overriding `output.fps`.
    pub fps: Option<u32>,
}

impl BuildOptions {
    /// Where a build writes when nobody says otherwise.
    pub fn defaults(project: &Project, script: &Path, locale: &str) -> Self {
        let stem = script
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "video".into());
        Self {
            // `build/` is what `new` gitignores, and a render is entirely
            // derived: it belongs where a checkout will not offer to commit
            // it. The name follows the committed timelines' — `cli.en.json`
            // beside `cli.en.mp4`.
            out: project
                .root
                .join("build")
                .join(format!("{stem}.{locale}.mp4")),
            narration_root: project.root.join("build").join("narration"),
            clips_dir: cache_root(project).join("video"),
            resolution: None,
            fps: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BuildReport {
    /// Always true. A failed build returns `BuildError` and prints the
    /// shared `ErrorReport`, so a consumer can branch on one key across
    /// every command rather than on this command's shape.
    pub ok: bool,
    pub output: PathBuf,
    pub duration_ms: u64,
    /// Which renderer did the work, for a report that says how it was made.
    pub renderer: &'static str,
    pub segments: usize,
    pub beats: usize,
    /// Beats that rendered as a slate because nothing had captured them.
    pub slates: usize,
    pub warnings: Vec<String>,
}

impl BuildReport {
    /// The human report. Warnings are not in it: `main` prints those to
    /// stderr for every command, and printing them here too said
    /// everything twice.
    pub fn render(&self) -> String {
        format!(
            "  {}\n  {} via {}\n  {} segment(s), {} beat(s), {} slate(s)\n",
            self.output.display(),
            clock(self.duration_ms),
            self.renderer,
            self.segments,
            self.beats,
            self.slates,
        )
    }
}

/// `m:ss.mmm`, the way a player would show it.
fn clock(ms: u64) -> String {
    format!("{}:{:02}.{:03}", ms / 60_000, (ms / 1000) % 60, ms % 1000)
}

pub enum BuildError {
    /// The script does not compile. Exit 2, as everywhere else.
    Validation(Vec<String>),
    /// Everything that is not the script's fault: a missing ffmpeg, a
    /// failed render, an unwritable output.
    Runtime(String),
}

/// One printable string for a failure, whichever kind it is.
pub fn render_error(error: &BuildError) -> String {
    match error {
        BuildError::Validation(diagnostics) => diagnostics.join("\n"),
        BuildError::Runtime(message) => message.clone(),
    }
}

/// Dub, then render what was dubbed.
pub async fn run_build(
    project: &Project,
    script: &Path,
    locale: &str,
    options: &BuildOptions,
) -> Result<BuildReport, BuildError> {
    run_build_with(
        &FfmpegRenderer::default(),
        project,
        script,
        locale,
        options,
        &mut |_| {},
    )
    .await
}

/// `run_build` with the renderer and the progress sink named.
///
/// The renderer is a parameter because there is to be more than one — the
/// ffmpeg path handles everything, and a pure-Rust path can handle scripts
/// whose beats are all hard cuts.
pub async fn run_build_with(
    renderer: &dyn Renderer,
    project: &Project,
    script: &Path,
    locale: &str,
    options: &BuildOptions,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<BuildReport, BuildError> {
    let dubbed = dub::run_dub(project, script, locale, &options.narration_root, false)
        .await
        .map_err(|e| match e {
            DubError::Validation(diagnostics) => BuildError::Validation(diagnostics),
            DubError::Runtime(message) => BuildError::Runtime(message),
        })?;

    // The script decides its own shape; a flag overrides it for this one
    // render, which is what makes a quick low-resolution check possible
    // without editing the script to get it.
    let (width, height) = options.resolution.unwrap_or(dubbed.output.resolution);
    let fps = options.fps.unwrap_or(dubbed.output.fps);
    let (render_plan, mut warnings) = plan::from_manifest(
        &dubbed.manifest,
        &Inputs {
            narration_dir: options.narration_root.join(locale),
            clips_dir: options.clips_dir.clone(),
            output: options.out.clone(),
            width,
            height,
            fps,
        },
    );
    let slates = render_plan
        .beats
        .iter()
        .filter(|b| b.picture == Picture::Slate)
        .count();

    let rendered = renderer
        .render(&render_plan, on_progress)
        .map_err(|e| BuildError::Runtime(explain(&e)))?;

    let mut all = dubbed.warnings;
    all.append(&mut warnings);
    Ok(BuildReport {
        ok: true,
        output: rendered.path,
        duration_ms: rendered.duration_ms,
        renderer: renderer.id(),
        segments: dubbed.manifest.segments.len(),
        beats: dubbed.manifest.beats.len(),
        slates,
        warnings: all,
    })
}

/// A render failure, with the one thing the author can act on in front.
fn explain(error: &RenderError) -> String {
    match error {
        RenderError::Unavailable { program, .. } => format!(
            "{program} is not on PATH. Rendering needs it; `teleprompt doctor` \
             reports what this build can see."
        ),
        other => other.to_string(),
    }
}

/// `WIDTHxHEIGHT`, as `--resolution` takes it.
///
/// Rejects rather than rounds: a typo'd resolution silently rendered at
/// 1920x1080 is a long wait for the wrong file.
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
