//! `build`: the whole pipeline, ending in a file you can play. It renders
//! from the manifest `dub` just published, as an outside consumer would
//! (docs/design.md#rendering).

use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_manifest::{captions, chapters, NarrationManifest};
use teleprompt_render::incremental::IncrementalRenderer;
use teleprompt_render::plan::{self, Inputs};
use teleprompt_render::{Picture, Progress, RenderError};

use crate::cmd::cache;
use crate::cmd::capture;
use crate::cmd::dub::{self, DubError};
use crate::project::Project;

/// Everything `build` needs beyond the script itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    /// The video to write.
    pub out: PathBuf,
    /// Where narration is published on the way past. A real directory, not
    /// a temporary one, so the render can be checked against its manifest.
    pub narration_root: PathBuf,
    /// Where captured clips are looked up, by shot hash.
    pub clips_dir: PathBuf,
    /// A frame size from the command line, overriding `output.resolution`.
    pub resolution: Option<(u32, u32)>,
    /// A frame rate from the command line, overriding `output.fps`.
    pub fps: Option<u32>,
    /// The compose cache (docs/design.md#compose-cache). `None` re-encodes
    /// the whole video every time.
    pub compose_dir: Option<PathBuf>,
    /// Megabytes of encoded video to keep afterwards, least recently used
    /// evicted first. Every build prunes, so the cap holds without anyone
    /// remembering to run a command.
    pub cache_max_mb: u64,
}

impl BuildOptions {
    /// Where a build writes when nobody says otherwise.
    pub fn defaults(project: &Project, script: &Path, locale: &str) -> Self {
        let stem = script
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "video".into());
        Self {
            // `build/` is what `new` gitignores. The name follows the
            // committed timelines': `cli.en.json` beside `cli.en.mp4`.
            out: project
                .root
                .join("build")
                .join(format!("{stem}.{locale}.mp4")),
            narration_root: project.root.join("build").join("narration"),
            clips_dir: project.caches().clips(),
            resolution: None,
            fps: None,
            compose_dir: Some(project.caches().compose()),
            cache_max_mb: cache::DEFAULT_MAX_MB,
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

pub enum BuildError {
    /// The script does not compile. Exit 2, as everywhere else.
    Validation(Vec<String>),
    /// Not the script's fault: a missing ffmpeg, a failed render, an
    /// unwritable output.
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
    run_build_with_capture(
        &renderer(options),
        &crate::scene::captures(),
        project,
        script,
        locale,
        options,
        &mut |_| {},
    )
    .await
}

/// The renderer a set of options asks for. `--no-cache` (no `compose_dir`)
/// switches off reuse, not the renderer, so both encode through one path.
pub fn renderer(options: &BuildOptions) -> IncrementalRenderer {
    IncrementalRenderer {
        program: "ffmpeg".into(),
        cache_dir: options
            .compose_dir
            .clone()
            .unwrap_or_else(std::env::temp_dir),
        reuse: options.compose_dir.is_some(),
    }
}

/// `run_build` with the renderer and the progress sink named.
pub async fn run_build_with(
    renderer: &IncrementalRenderer,
    project: &Project,
    script: &Path,
    locale: &str,
    options: &BuildOptions,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<BuildReport, BuildError> {
    run_build_with_capture(
        renderer,
        &crate::scene::captures(),
        project,
        script,
        locale,
        options,
        on_progress,
    )
    .await
}

/// `run_build_with`, with the capture backends named too.
///
/// A parameter so a test can build a machine that cannot record a scene,
/// and check the warning and slate that follow.
pub async fn run_build_with_capture(
    renderer: &IncrementalRenderer,
    captures: &teleprompt_capture::CaptureRegistry,
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

    let (width, height) = options.resolution.unwrap_or(dubbed.output.resolution);
    let fps = options.fps.unwrap_or(dubbed.output.fps);

    // Stage 5 (docs/design.md#pipeline). What cannot be recorded becomes a
    // warning and a slate, not a failure; see [`capture::run_capture`].
    let mut warnings = dubbed.warnings;
    let recorded = capture::run_capture(
        &dubbed.manifest,
        &dubbed.shots,
        &dubbed.scenes,
        captures,
        &options.clips_dir,
        teleprompt_capture::Frame { width, height, fps },
        &mut |_| {},
    );
    warnings.extend(recorded.warnings);
    let captured = recorded.captured;

    let (render_plan, mut plan_warnings) = plan::from_manifest(
        &dubbed.manifest,
        &Inputs {
            narration_dir: options.narration_root.join(locale),
            clips_dir: options.clips_dir.clone(),
            output: options.out.clone(),
            width,
            height,
            fps,
            names: dubbed.output.names,
        },
    );
    let slates = render_plan
        .shots
        .iter()
        .filter(|b| b.picture == Picture::Slate)
        .count();

    let rendered = renderer
        .render(&render_plan, on_progress)
        .map_err(|e| BuildError::Runtime(explain(&e)))?;

    // Pruned after the render, never before: pruning first could evict
    // pieces this render was about to copy.
    warnings.append(&mut plan_warnings);
    let mut all = warnings;
    all.extend(prune_compose_cache(options));

    let captions = write_captions(&rendered.path, &dubbed.manifest)?;
    let chapters = write_chapters(&rendered.path, &dubbed.manifest, &mut all)?;

    Ok(BuildReport {
        ok: true,
        output: rendered.path,
        captions,
        chapters,
        duration_ms: rendered.duration_ms,
        renderer: renderer.id(),
        lines: dubbed.manifest.lines.len(),
        items: dubbed.manifest.shots.len(),
        slates,
        captured,
        reused_ms: rendered.reused_ms,
        warnings: all,
    })
}

/// Shrinks the compose cache to `cache_max_mb`; what went wrong, if it did.
/// A cache that will not shrink is a disk problem, not a reason to report a
/// video that exists as a failure.
fn prune_compose_cache(options: &BuildOptions) -> Option<String> {
    let dir = options.compose_dir.as_ref()?;
    let limit = options.cache_max_mb * 1_048_576;
    cache::prune(dir, limit).err().map(|e| {
        format!(
            "could not prune the compose cache at {}: {e}",
            dir.display()
        )
    })
}

/// YouTube chapter timestamps beside the video; what keeps them from
/// counting as chapters goes into `warnings`, when there are any to show.
fn write_chapters(
    video: &Path,
    manifest: &NarrationManifest,
    warnings: &mut Vec<String>,
) -> Result<PathBuf, BuildError> {
    let (list, problems) = chapters::youtube(manifest);
    let path = video.with_extension("chapters.txt");
    std::fs::write(&path, list)
        .map_err(|e| BuildError::Runtime(format!("cannot write {}: {e}", path.display())))?;
    if manifest.chapters.len() > 1 {
        warnings.extend(problems);
    }
    Ok(path)
}

/// The video's subtitles, named after it (`tour.en.srt` beside
/// `tour.en.mp4`), where players look for them.
fn write_captions(video: &Path, manifest: &NarrationManifest) -> Result<Vec<PathBuf>, BuildError> {
    let cues = captions::cues(manifest);
    [("srt", captions::srt(&cues)), ("vtt", captions::vtt(&cues))]
        .into_iter()
        .map(|(ext, text)| {
            let path = video.with_extension(ext);
            std::fs::write(&path, text)
                .map(|()| path.clone())
                .map_err(|e| BuildError::Runtime(format!("cannot write {}: {e}", path.display())))
        })
        .collect()
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

impl From<BuildError> for crate::output::Outcome {
    fn from(e: BuildError) -> Self {
        match e {
            BuildError::Validation(errors) => Self::ValidationError(errors),
            BuildError::Runtime(message) => Self::RuntimeFailure(message),
        }
    }
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
