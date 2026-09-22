//! Rendering by encoding only the parts that moved.
//!
//! The monolithic renderer re-encodes every frame of a video to change one
//! sentence of it, which on a two-minute script is most of a warm build.
//! This one encodes [`chunks`](crate::chunk) separately, caches each
//! under a key covering everything that changes its bytes, and stitches
//! them with the `concat` demuxer and `-c copy` — which copies compressed
//! frames rather than decoding and re-encoding them, and costs roughly
//! nothing.
//!
//! The audio is still mixed in one pass. It is cheap (a four-hundred-second
//! mix and AAC encode measured 1.4s against 7.3s for the same picture), it
//! has no natural seams, and a mix cut into cached placements would put a join
//! in the middle of a word.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use teleprompt_core::Hash;

use crate::chunk::{self, Chunk, ChunkKey, Content, Source, Window};
use crate::ffmpeg;
use crate::placement::{xfade_for, BACKGROUND};
use crate::{Progress, RenderError, RenderPlan, Rendered, Renderer};

/// Renders through a cache of encoded chunks.
#[derive(Debug, Clone)]
pub struct IncrementalRenderer {
    /// The binary to invoke. Configurable because `doctor` may have found a
    /// usable ffmpeg somewhere other than `PATH`.
    pub program: String,
    /// Where encoded chunks are kept. Entirely derived — everything in
    /// it can be reproduced from the key that names it — so it belongs
    /// wherever the rest of the cache does.
    pub cache_dir: PathBuf,
    /// Whether a chunk already in the cache may be used.
    ///
    /// `false` is `--no-cache`: re-encode everything. It still writes what
    /// it encodes, because the point is to distrust what is there, not to
    /// refuse to leave anything behind. This is a flag rather than a second
    /// renderer so that `--no-cache` and an ordinary build come out of the
    /// same code — two renderers agree until they do not.
    pub reuse: bool,
}

impl Renderer for IncrementalRenderer {
    fn id(&self) -> &'static str {
        "ffmpeg-incremental"
    }

    fn render(
        &self,
        plan: &RenderPlan,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Rendered, RenderError> {
        // The only plan that cannot be cut is a shot shorter than the
        // transitions either side of it, and the scheduler caps a
        // transition against what the item it arrives in has left, so this
        // is unreachable from a real script. It used to hand the build to a
        // second renderer without saying so; a fallback nobody can see is
        // how two implementations drift apart.
        let Some(chunks) = chunk::chunks(plan) else {
            return Err(RenderError::Unchunkable);
        };

        ffmpeg::ensure_parent(&plan.output)?;
        std::fs::create_dir_all(&self.cache_dir).map_err(|source| RenderError::Io {
            path: self.cache_dir.display().to_string(),
            source,
        })?;

        // One read per distinct clip, however many chunks draw on it: a
        // transition alone puts the same file in three of them.
        let mut identities: HashMap<PathBuf, Hash> = HashMap::new();

        // Reuse is reported against the output's own clock rather than in
        // frames: "five and a half minutes of this came from the cache" is
        // a thing an author can check against the video in front of them.
        let total_frames: u64 = chunks.iter().map(|s| s.frames).sum();
        let on_the_clock = |frames: u64| match total_frames {
            0 => 0,
            total => frames * plan.duration_ms / total,
        };

        let mut list = String::new();
        let mut done_frames = 0u64;
        let mut reused_frames = 0u64;
        for chunk in &chunks {
            let key = ChunkKey::for_chunk(plan, chunk)
                .hash(&mut |path| identity(&mut identities, path))
                .map_err(|source| RenderError::Io {
                    path: "a captured clip".into(),
                    source,
                })?;
            let cached = self.cache_dir.join(format!("{key}.mp4"));

            if self.reuse && is_usable(&cached) {
                reused_frames += chunk.frames;
                mark_used(&cached);
            } else {
                // Encoded beside the entry and renamed into place, which is
                // atomic on one filesystem. A render killed halfway through
                // otherwise leaves a truncated mp4 under a key that says it
                // is complete, and every later build serves it.
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
            }

            // Single quotes are the concat demuxer's own escape, and a
            // cache path is ours rather than the author's — but the output
            // directory it sits under is not.
            list.push_str(&format!(
                "file '{}'\n",
                cached.display().to_string().replace('\'', r"'\''")
            ));

            done_frames += chunk.frames;
            on_progress(Progress {
                rendered_ms: on_the_clock(done_frames),
                of_ms: plan.duration_ms,
            });
        }

        let list_path = self.cache_dir.join(format!(
            ".concat-{}.txt",
            Hash::of(plan.output.display().to_string().as_bytes()).short()
        ));
        std::fs::write(&list_path, &list).map_err(|source| RenderError::Io {
            path: list_path.display().to_string(),
            source,
        })?;

        let result = ffmpeg::run(
            &self.program,
            &assemble_args(plan, &list_path),
            &mut |rendered_ms| {
                on_progress(Progress {
                    rendered_ms,
                    of_ms: plan.duration_ms,
                });
            },
        );
        let _ = std::fs::remove_file(&list_path);
        result?;

        on_progress(Progress {
            rendered_ms: plan.duration_ms,
            of_ms: plan.duration_ms,
        });
        Ok(Rendered {
            path: plan.output.clone(),
            duration_ms: plan.duration_ms,
            reused_ms: Some(on_the_clock(reused_frames)),
        })
    }
}

/// A clip's identity is its contents, read once per render.
fn identity(seen: &mut HashMap<PathBuf, Hash>, path: &Path) -> std::io::Result<Hash> {
    if let Some(hash) = seen.get(path) {
        return Ok(*hash);
    }
    let hash = Hash::of(&std::fs::read(path)?);
    seen.insert(path.to_path_buf(), hash);
    Ok(hash)
}

/// Record that an entry was copied from, by setting its modification time.
///
/// This is the only thing that distinguishes a chunk three builds have
/// leaned on from one nothing has wanted since April, and a cache with a
/// size cap has to evict the second. Best effort: a read-only cache
/// directory is a reason to render more slowly next time, not a reason to
/// fail a render that has already succeeded.
fn mark_used(path: &Path) {
    let _ = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|f| {
            f.set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now()))
        });
}

/// Whether a cache entry can be served. An empty file is a crashed encode,
/// not a chunk of no frames — those are never written.
fn is_usable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// Seconds, to the microsecond. Frame counts are exact in this unit up to
/// far more frames than a video has.
fn seconds_of(frames: u64, fps: u32) -> String {
    format!("{:.6}", frames as f64 / f64::from(fps))
}

/// The argv that encodes one chunk to a video-only file.
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
            // Half a frame short of the full window so the blend can never
            // ask for more than either input holds, and trimmed back to the
            // exact frame count after — the chunk's length is arithmetic
            // the concat depends on, not something to leave to rounding.
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

/// One window as an ffmpeg input and the filter chain that cuts it out.
fn window_input(
    plan: &RenderPlan,
    window: &Window,
    index: usize,
    label: &str,
    args: &mut Vec<String>,
    filters: &mut Vec<String>,
) {
    // Long enough that the chain never runs out of source before the
    // `trim` below decides where the window ends.
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
    // `tpad` clones the first and last frames rather than filling with a
    // colour, which is what makes a held gap a freeze instead of a cut to
    // black. It pads generously at the end and `trim` sets the exact
    // window — in frames, because frames are what has to add up.
    // `settb=AVTB` because `xfade` refuses two inputs whose timebases
    // differ, and its own output differs from its inputs'.
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

/// The encoder settings, in one place: [`chunk::RECIPE`] names this, and
/// a change here that is not a change there serves stale frames.
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

/// The argv that concatenates the encoded chunks and mixes the narration
/// over them.
///
/// The picture is copied, not re-encoded: `-c:v copy` on the concat
/// demuxer's output moves compressed frames straight into the container.
fn assemble_args(plan: &RenderPlan, list: &Path) -> Vec<String> {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-y".into()];
    args.extend([
        "-f".into(),
        "concat".into(),
        // The list holds absolute paths into the cache, which `-safe 1`
        // — the default — refuses.
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list.display().to_string(),
    ]);

    // A silent bed the narration is mixed over, so the output has a
    // continuous audio stream even where nobody is speaking.
    let bed = 1;
    args.extend([
        "-f".into(),
        "lavfi".into(),
        "-t".into(),
        crate::placement::seconds(plan.duration_ms),
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
        // `adelay` takes milliseconds — the unit the manifest publishes —
        // so no rounding happens here.
        let input = bed + 1 + i;
        filters.push(format!(
            "[{input}:a]adelay={ms}|{ms}[a{input}]",
            ms = clip.start_ms
        ));
        mix.push(format!("[a{input}]"));
    }
    // `normalize=0`: amix otherwise divides every input by the number of
    // inputs, so a script's narration would get quieter the more chunks
    // it had.
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
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "128k".into(),
        "-movflags".into(),
        "+faststart".into(),
    ]);
    args.push(plan.output.display().to_string());
    args
}
