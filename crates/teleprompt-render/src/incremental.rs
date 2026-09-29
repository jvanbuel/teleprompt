//! Rendering by encoding only the [`chunks`](crate::chunk) that changed,
//! then mixing the audio in one pass (`docs/design.md#compose-cache`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use teleprompt_core::Hash;

use crate::chunk::{self, Chunk, ChunkKey, Content, Source, Window};
use crate::ffmpeg;
use crate::placement::{xfade_for, BACKGROUND};
use crate::{Progress, RenderError, RenderPlan, Rendered};

/// Renders through a cache of encoded chunks.
#[derive(Debug, Clone)]
pub struct IncrementalRenderer {
    /// The ffmpeg binary.
    pub program: String,
    /// Where encoded chunks are kept; entirely derived.
    pub cache_dir: PathBuf,
    /// `false` is `--no-cache`: re-encode everything, but still write what
    /// is encoded. A flag, not a second renderer, so both paths share code.
    pub reuse: bool,
}

impl IncrementalRenderer {
    /// Stable identifier, for reporting which path a render took.
    pub fn id(&self) -> &'static str {
        "ffmpeg-incremental"
    }

    pub fn render(
        &self,
        plan: &RenderPlan,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Rendered, RenderError> {
        let Some(chunks) = chunk::chunks(plan) else {
            return Err(RenderError::Unchunkable);
        };

        ffmpeg::ensure_parent(&plan.output)?;
        std::fs::create_dir_all(&self.cache_dir).map_err(|source| RenderError::Io {
            path: self.cache_dir.display().to_string(),
            source,
        })?;

        // One read per distinct clip; a transition puts one in three chunks.
        let mut identities: HashMap<PathBuf, Hash> = HashMap::new();

        // Reported in output time, which an author can check against the
        // video, rather than in frames.
        let total_frames: u64 = chunks.iter().map(|s| s.frames).sum();
        let on_the_clock = |frames: u64| match total_frames {
            0 => 0,
            total => frames * plan.duration_ms.ms() / total,
        };

        // The concat pass counts from zero again, so progress is clamped
        // to never go backwards.
        let mut furthest = 0u64;
        let mut report = |on_progress: &mut dyn FnMut(Progress), rendered_ms: u64| {
            furthest = furthest.max(rendered_ms);
            on_progress(Progress {
                rendered_ms: furthest,
                of_ms: plan.duration_ms.ms(),
            });
        };

        let mut list = String::new();
        let mut done_frames = 0u64;
        let mut reused_frames = 0u64;
        for chunk in &chunks {
            let (cached, reused) = self.chunk_file(plan, chunk, &mut identities)?;
            if reused {
                reused_frames += chunk.frames;
            }

            // The concat demuxer's quoting: the cache directory is the
            // author's choice and may contain a quote.
            list.push_str(&format!(
                "file '{}'\n",
                cached.display().to_string().replace('\'', r"'\''")
            ));

            done_frames += chunk.frames;
            report(on_progress, on_the_clock(done_frames));
        }

        self.assemble(plan, &list, &mut |rendered_ms| {
            report(on_progress, rendered_ms);
        })?;

        on_progress(Progress {
            rendered_ms: plan.duration_ms.ms(),
            of_ms: plan.duration_ms.ms(),
        });
        Ok(Rendered {
            path: plan.output.clone(),
            duration_ms: plan.duration_ms.ms(),
            reused_ms: self.reuse.then(|| on_the_clock(reused_frames)),
        })
    }

    /// The chunk's cached encoding, encoding it first unless reuse is on
    /// and it is there; and whether it was reused.
    fn chunk_file(
        &self,
        plan: &RenderPlan,
        chunk: &Chunk,
        identities: &mut HashMap<PathBuf, Hash>,
    ) -> Result<(PathBuf, bool), RenderError> {
        let key = ChunkKey::for_chunk(plan, chunk)
            .hash(&mut |path| identity(identities, path))
            .map_err(|source| RenderError::Io {
                path: "a captured clip".into(),
                source,
            })?;
        let cached = self.cache_dir.join(format!("{key}.mp4"));

        if self.reuse && is_usable(&cached) {
            mark_used(&cached);
            return Ok((cached, true));
        }
        // Encoded beside the entry and renamed into place (atomic on
        // one filesystem), or a killed render leaves a truncated mp4
        // that every later build serves.
        let partial = self.cache_dir.join(format!(".{key}.partial.mp4"));
        ffmpeg::run(
            &self.program,
            &encode_args(plan, chunk, &partial),
            &mut |_| {},
        )?;
        std::fs::rename(&partial, &cached).map_err(|source| RenderError::Io {
            path: cached.display().to_string(),
            source,
        })?;
        Ok((cached, false))
    }

    /// Copies the chunks `list` names into the output, with the narration.
    fn assemble(
        &self,
        plan: &RenderPlan,
        list: &str,
        on_out_ms: &mut dyn FnMut(u64),
    ) -> Result<(), RenderError> {
        let list_path = self.cache_dir.join(format!(
            ".concat-{}.txt",
            Hash::of(plan.output.display().to_string().as_bytes()).short()
        ));
        std::fs::write(&list_path, list).map_err(|source| RenderError::Io {
            path: list_path.display().to_string(),
            source,
        })?;

        // Like a chunk: a render killed part way leaves the last good video.
        let partial = partial_beside(&plan.output);
        let result = ffmpeg::run(
            &self.program,
            &assemble_args(plan, &list_path, &partial),
            on_out_ms,
        )
        .and_then(|()| {
            std::fs::rename(&partial, &plan.output).map_err(|source| RenderError::Io {
                path: plan.output.display().to_string(),
                source,
            })
        });
        let _ = std::fs::remove_file(&list_path);
        if result.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        result
    }
}

/// Keyed on contents; see [`ChunkKey::hash`].
fn identity(seen: &mut HashMap<PathBuf, Hash>, path: &Path) -> std::io::Result<Hash> {
    if let Some(hash) = seen.get(path) {
        return Ok(*hash);
    }
    let hash = Hash::of_file(path)?;
    seen.insert(path.to_path_buf(), hash);
    Ok(hash)
}

/// Touches the modification time, which least-recently-used eviction
/// reads. Best effort: a read-only cache must not fail a render.
fn mark_used(path: &Path) {
    let _ = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|f| {
            f.set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now()))
        });
}

/// An empty file is a crashed encode; empty chunks are never written.
fn is_usable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// Seconds to the microsecond, exact for any realistic frame count.
fn seconds_of(frames: u64, fps: u32) -> String {
    format!("{:.6}", frames as f64 / f64::from(fps))
}

fn encode_args(plan: &RenderPlan, chunk: &Chunk, out: &Path) -> Vec<String> {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-y".into()];
    let mut filters: Vec<String> = Vec::new();

    match &chunk.content {
        Content::Body(window) => {
            window_input(plan, window, 0, "v", &mut args, &mut filters);
        }
        Content::Blend { kind, from, to } => {
            window_input(plan, from, 0, "a", &mut args, &mut filters);
            window_input(plan, to, 1, "b", &mut args, &mut filters);
            // Half a frame short so the blend never asks for more than an
            // input holds, then trimmed to the exact count the concat needs.
            let blend = format!("{:.6}", (chunk.frames as f64 - 0.5) / f64::from(plan.fps));
            filters.push(format!(
                "[a][b]xfade=transition={}:duration={blend}:offset=0[x];\
                 [x]trim=end_frame={},setpts=PTS-STARTPTS,settb=AVTB[v]",
                xfade_for(kind),
                chunk.frames,
            ));
        }
    }

    args.push("-filter_complex".into());
    args.push(filters.join(";"));
    args.extend(["-map".into(), "[v]".into(), "-an".into()]);
    args.extend(encoder(plan.fps));
    args.extend(["-frames:v".into(), chunk.frames.to_string()]);
    args.push(out.display().to_string());
    args
}

fn window_input(
    plan: &RenderPlan,
    window: &Window,
    index: usize,
    label: &str,
    args: &mut Vec<String>,
    filters: &mut Vec<String>,
) {
    // Enough that the source outlasts the `trim` below.
    let enough = seconds_of(
        window.from_frame + window.frames + u64::from(plan.fps),
        plan.fps,
    );
    match &window.source {
        Source::Clip(path) => {
            args.push("-i".into());
            args.push(path.display().to_string());
        }
        Source::Slate => {
            args.extend(["-f".into(), "lavfi".into(), "-t".into(), enough.clone()]);
            args.push("-i".into());
            args.push(format!(
                "color=c={BACKGROUND}:s={}x{}:r={}",
                plan.width, plan.height, plan.fps
            ));
        }
    }
    // `tpad` clones edge frames, so a held gap is a freeze, not black;
    // `trim` then cuts the exact window in frames. `settb=AVTB` because
    // `xfade` refuses inputs whose timebases differ.
    filters.push(format!(
        "[{index}:v]scale={w}:{h}:force_original_aspect_ratio=decrease,\
         pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color={BACKGROUND},setsar=1,fps={fps},\
         tpad=start_mode=clone:start_duration={lead}:stop_mode=clone:stop_duration={enough},\
         trim=start_frame={from}:end_frame={end},setpts=PTS-STARTPTS,settb=AVTB[{label}]",
        w = plan.width,
        h = plan.height,
        fps = plan.fps,
        lead = seconds_of(window.lead_in_frames, plan.fps),
        from = window.from_frame,
        end = window.from_frame + window.frames,
    ));
}

/// Changing this without bumping [`chunk::RECIPE`] serves stale frames.
fn encoder(fps: u32) -> Vec<String> {
    let mut out: Vec<String> = [
        "-c:v", "libx264", "-preset", "medium", "-crf", "23", "-pix_fmt", "yuv420p", "-r",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    out.push(fps.to_string());
    out
}

/// Concatenates the chunks with `-c:v copy` (no re-encode) and mixes the
/// narration over them.
fn assemble_args(plan: &RenderPlan, list: &Path, output: &Path) -> Vec<String> {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-y".into()];
    args.extend([
        "-f".into(),
        "concat".into(),
        // The default `-safe 1` refuses the list's absolute paths.
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list.display().to_string(),
    ]);

    // A silent bed keeps the audio stream continuous between lines.
    let bed = 1;
    args.extend([
        "-f".into(),
        "lavfi".into(),
        "-t".into(),
        teleprompt_core::time::ffmpeg_seconds(plan.duration_ms.ms()),
        "-i".into(),
        format!(
            "anullsrc=channel_layout=mono:sample_rate={}",
            ffmpeg::SAMPLE_RATE
        ),
    ]);

    let mut filters: Vec<String> = Vec::new();
    let mut mix: Vec<String> = vec![format!("[{bed}:a]")];
    for (i, clip) in plan.narration.iter().enumerate() {
        args.push("-i".into());
        args.push(clip.path.display().to_string());
        // Milliseconds, as published, so nothing is rounded.
        let input = bed + 1 + i;
        filters.push(format!(
            "[{input}:a]adelay={ms}|{ms}[a{input}]",
            ms = clip.start_ms.ms()
        ));
        mix.push(format!("[a{input}]"));
    }
    // `normalize=0`, or amix scales each input down by the input count and
    // narration gets quieter the more lines a script has.
    filters.push(format!(
        "{}amix=inputs={}:normalize=0:dropout_transition=0[a]",
        mix.join(""),
        mix.len()
    ));

    args.push("-filter_complex".into());
    args.push(filters.join(";"));
    args.extend([
        "-map".into(),
        "0:v".into(),
        "-c:v".into(),
        "copy".into(),
        "-map".into(),
        "[a]".into(),
    ]);
    ffmpeg::audio_encode(&mut args);
    args.push(output.display().to_string());
    args
}

/// `out.mp4` → `.out.partial.mp4` in the same directory, so the rename is
/// atomic and ffmpeg still picks the container from the extension.
fn partial_beside(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .map_or_else(Default::default, |n| n.to_string_lossy());
    let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, "mp4"));
    output.with_file_name(format!(".{stem}.partial.{ext}"))
}
