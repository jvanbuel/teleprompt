//! The ffmpeg renderer: a filter graph and an argument vector.
//!
//! ffmpeg is invoked as a subprocess with an explicit argument vector,
//! never a shell string and never linked. Linking libav would put clang and
//! the libav headers in every build, and a distro ffmpeg built
//! `--enable-gpl` inside an MIT binary. Invoking a subprocess is neither.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

use crate::{Picture, Progress, RenderError, RenderPlan, Rendered, Renderer};

/// The colour a slate holds, and what a clip narrower than the frame is
/// padded with.
const BACKGROUND: &str = "0x0b0d10";

/// The narration bed's sample rate. Matches what the voice crates emit, so
/// mixing never resamples.
const SAMPLE_RATE: u32 = 24_000;

/// One continuous run of picture: a beat, or the hold that covers the time
/// between two of them.
///
/// The plan's beats do not tile the timeline. Narration opens after a
/// lead-in, so the first beat starts late; a script can end on speech with
/// no action under it, so the last one ends early. Those gaps are picture
/// too, and something has to fill them or every beat after a gap renders
/// early against audio that is still correctly placed.
struct Piece {
    duration_ms: u64,
    picture: Picture,
}

/// The plan's beats plus the holds between them, in order, covering
/// `[0, plan.duration_ms)` exactly.
fn pieces(plan: &RenderPlan) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut cursor = 0u64;
    let mut hold = |out: &mut Vec<Piece>, until: u64, cursor: u64| {
        if until > cursor {
            out.push(Piece {
                duration_ms: until - cursor,
                picture: Picture::Slate,
            });
        }
    };

    for beat in &plan.beats {
        hold(&mut out, beat.start_ms, cursor);
        out.push(Piece {
            duration_ms: beat.duration_ms,
            picture: beat.picture.clone(),
        });
        cursor = beat.start_ms + beat.duration_ms;
    }
    hold(&mut out, plan.duration_ms, cursor);
    out
}

/// The argument vector for `plan`, ffmpeg's own name excluded.
///
/// Pure, so the graph can be asserted on without ffmpeg installed.
pub fn args(plan: &RenderPlan) -> Vec<String> {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-y".into()];
    let mut filters: Vec<String> = Vec::new();
    let pieces = pieces(plan);

    // Picture inputs come first, one per piece, so a piece's input index is
    // its position.
    for piece in &pieces {
        match &piece.picture {
            Picture::Clip(path) => {
                args.push("-i".into());
                args.push(path.display().to_string());
            }
            Picture::Slate => {
                args.push("-f".into());
                args.push("lavfi".into());
                args.push("-t".into());
                args.push(seconds(piece.duration_ms));
                args.push("-i".into());
                args.push(format!(
                    "color=c={BACKGROUND}:s={}x{}:r={}",
                    plan.width, plan.height, plan.fps
                ));
            }
        }
    }

    // A silent bed the narration is mixed over, so the output has a
    // continuous audio stream even where nobody is speaking.
    let bed = pieces.len();
    args.push("-f".into());
    args.push("lavfi".into());
    args.push("-t".into());
    args.push(seconds(plan.duration_ms));
    args.push("-i".into());
    args.push(format!(
        "anullsrc=channel_layout=mono:sample_rate={SAMPLE_RATE}"
    ));

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
    // inputs, so a script's narration would get quieter the more segments
    // it had.
    filters.push(format!(
        "{}amix=inputs={}:normalize=0:dropout_transition=0[a]",
        mix.join(""),
        mix.len()
    ));

    // Every piece is normalized to the same size, rate and duration before
    // anything is joined: ffmpeg's concat filter requires it, and a clip
    // recorded at another size would otherwise fail the render rather than
    // be fitted.
    for (i, piece) in pieces.iter().enumerate() {
        let d = seconds(piece.duration_ms);
        filters.push(format!(
            "[{i}:v]scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color={BACKGROUND},setsar=1,fps={fps},\
             tpad=stop_mode=clone:stop_duration={d},trim=duration={d},setpts=PTS-STARTPTS[v{i}]",
            w = plan.width,
            h = plan.height,
            fps = plan.fps,
        ));
    }
    let joined: String = (0..pieces.len()).map(|i| format!("[v{i}]")).collect();
    filters.push(format!("{joined}concat=n={}:v=1:a=0[v]", pieces.len()));

    args.push("-filter_complex".into());
    args.push(filters.join(";"));
    args.push("-map".into());
    args.push("[v]".into());
    args.push("-map".into());
    args.push("[a]".into());
    args.push("-c:v".into());
    args.push("libx264".into());
    args.push("-preset".into());
    args.push("medium".into());
    args.push("-crf".into());
    args.push("23".into());
    args.push("-pix_fmt".into());
    args.push("yuv420p".into());
    args.push("-r".into());
    args.push(plan.fps.to_string());
    args.push("-c:a".into());
    args.push("aac".into());
    args.push("-b:a".into());
    args.push("128k".into());
    args.push("-movflags".into());
    args.push("+faststart".into());
    args.push(plan.output.display().to_string());
    args
}

/// Milliseconds as seconds, which is the unit ffmpeg's `-t` takes.
fn seconds(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// Renders by invoking `ffmpeg`.
#[derive(Debug, Clone)]
pub struct FfmpegRenderer {
    /// The binary to invoke. Configurable because `doctor` may have found a
    /// usable ffmpeg somewhere other than `PATH`.
    pub program: String,
}

impl Default for FfmpegRenderer {
    fn default() -> Self {
        Self {
            program: "ffmpeg".into(),
        }
    }
}

impl Renderer for FfmpegRenderer {
    fn id(&self) -> &'static str {
        "ffmpeg"
    }

    fn render(
        &self,
        plan: &RenderPlan,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Rendered, RenderError> {
        if let Some(parent) = plan.output.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|source| RenderError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
        }

        // `-progress pipe:1` writes machine-readable `key=value` lines to
        // stdout, so nothing has to scrape the human-readable stderr — which
        // is a log, not an interface, and changes between ffmpeg releases.
        let mut command = Command::new(&self.program);
        command
            .args(["-nostdin", "-nostats", "-progress", "pipe:1"])
            .args(args(plan))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|source| RenderError::Unavailable {
            program: self.program.clone(),
            source,
        })?;

        // stderr is drained on its own thread. A render that fills the pipe
        // buffer while nobody reads it deadlocks, and ffmpeg is verbose
        // enough on a long script to do exactly that.
        let mut err = child.stderr.take().expect("stderr was piped");
        let draining = std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = err.read_to_string(&mut buf);
            buf
        });

        let out = child.stdout.take().expect("stdout was piped");
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            // `out_time_us` is microseconds of output written so far. The
            // older `out_time_ms` key is also microseconds despite its name,
            // which is a trap worth not walking into.
            if let Some(us) = line.strip_prefix("out_time_us=") {
                if let Ok(us) = us.trim().parse::<u64>() {
                    on_progress(Progress {
                        rendered_ms: us / 1000,
                        of_ms: plan.duration_ms,
                    });
                }
            }
        }

        let status = child.wait().map_err(|source| RenderError::Io {
            path: plan.output.display().to_string(),
            source,
        })?;
        let stderr = draining.join().unwrap_or_default();

        if !status.success() {
            return Err(RenderError::Failed {
                program: self.program.clone(),
                status: status.to_string(),
                stderr: tail(&stderr),
            });
        }

        on_progress(Progress {
            rendered_ms: plan.duration_ms,
            of_ms: plan.duration_ms,
        });
        Ok(Rendered {
            path: plan.output.clone(),
            duration_ms: plan.duration_ms,
        })
    }
}

/// ffmpeg's last words. The interesting line of a failure is the last one;
/// everything above it is the input report, which is long and, when the
/// graph is at fault, irrelevant.
fn tail(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().collect();
    let from = lines.len().saturating_sub(20);
    lines[from..].join("\n")
}
