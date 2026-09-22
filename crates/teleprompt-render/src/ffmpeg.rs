//! The ffmpeg renderer: a filter graph and an argument vector.
//!
//! ffmpeg is invoked as a subprocess with an explicit argument vector,
//! never a shell string and never linked. Linking libav would put clang and
//! the libav headers in every build, and a distro ffmpeg built
//! `--enable-gpl` inside an MIT binary. Invoking a subprocess is neither.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

use crate::piece::{pieces, seconds, xfade_for, BACKGROUND};
use crate::{Picture, Progress, RenderError, RenderPlan, Rendered, Renderer};

/// The narration bed's sample rate. Matches what the voice crates emit, so
/// mixing never resamples.
pub(crate) const SAMPLE_RATE: u32 = 24_000;

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
            // `Hold` never reaches here: `pieces` folds it into the piece
            // before it, which is what holding means.
            Picture::Hold | Picture::Slate => {
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

    // Every piece is normalized to the same size, rate, duration and
    // timebase before anything is joined: `concat` requires matching
    // formats, and `xfade` additionally refuses two inputs whose timebases
    // differ — which is what its own output does to the next piece in the
    // chain unless everything is pinned to `AVTB` first.
    for (i, piece) in pieces.iter().enumerate() {
        let d = seconds(piece.duration_ms);
        // `tpad` clones the first and last frames rather than filling with
        // a colour, which is what makes a gap a freeze instead of a cut to
        // black. The stop padding is deliberately longer than needed and
        // the `trim` after it sets the exact length: a clip longer than its
        // slot is cut, a shorter one holds.
        filters.push(format!(
            "[{i}:v]scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color={BACKGROUND},setsar=1,fps={fps},\
             tpad=start_mode=clone:start_duration={lead}:stop_mode=clone:stop_duration={d},\
             trim=duration={d},setpts=PTS-STARTPTS,settb=AVTB[v{i}]",
            w = plan.width,
            h = plan.height,
            fps = plan.fps,
            lead = seconds(piece.lead_in_ms),
        ));
    }
    // Joined pairwise rather than by one n-ary `concat`, because a blend
    // takes two streams and produces one: a chain handles both kinds of
    // join in one shape.
    let mut label = "[v0]".to_string();
    for (i, piece) in pieces.iter().enumerate().skip(1) {
        let out = format!("[j{i}]");
        match &piece.blend {
            Some((kind, overlap)) => {
                // `offset` is where the blend begins in the stream built so
                // far, which starts at zero because the pieces tile the
                // timeline from zero.
                filters.push(format!(
                    "{label}[v{i}]xfade=transition={}:duration={}:offset={}{out}",
                    xfade_for(kind),
                    seconds(*overlap),
                    seconds(piece.start_ms),
                ));
            }
            None => filters.push(format!("{label}[v{i}]concat=n=2:v=1:a=0{out}")),
        }
        label = out;
    }
    filters.push(format!("{label}null[v]"));

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
        ensure_parent(&plan.output)?;
        run(&self.program, &args(plan), &mut |rendered_ms| {
            on_progress(Progress {
                rendered_ms,
                of_ms: plan.duration_ms,
            });
        })?;
        on_progress(Progress {
            rendered_ms: plan.duration_ms,
            of_ms: plan.duration_ms,
        });
        Ok(Rendered {
            path: plan.output.clone(),
            duration_ms: plan.duration_ms,
            reused_ms: None,
        })
    }
}

/// Create the directory a render is about to write into.
pub(crate) fn ensure_parent(path: &std::path::Path) -> Result<(), RenderError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| RenderError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
    }
    Ok(())
}

/// Run ffmpeg to completion, reporting output time as it goes.
///
/// Shared by both renderers, because getting this wrong is not a graph bug
/// — it is a hang. ffmpeg is verbose enough on a long script to fill a pipe
/// buffer, and a render nobody is draining stops there for ever.
pub(crate) fn run(
    program: &str,
    args: &[String],
    on_out_time_ms: &mut dyn FnMut(u64),
) -> Result<(), RenderError> {
    // `-progress pipe:1` writes machine-readable `key=value` lines to
    // stdout, so nothing has to scrape the human-readable stderr — which
    // is a log, not an interface, and changes between ffmpeg releases.
    let mut child = Command::new(program)
        .args(["-nostdin", "-nostats", "-progress", "pipe:1"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| RenderError::Unavailable {
            program: program.to_string(),
            source,
        })?;

    // stderr is drained on its own thread. A render that fills the pipe
    // buffer while nobody reads it deadlocks.
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
                on_out_time_ms(us / 1000);
            }
        }
    }

    let status = child.wait().map_err(|source| RenderError::Io {
        path: program.to_string(),
        source,
    })?;
    let stderr = draining.join().unwrap_or_default();
    if !status.success() {
        return Err(RenderError::Failed {
            program: program.to_string(),
            status: status.to_string(),
            stderr: tail(&stderr),
        });
    }
    Ok(())
}

/// ffmpeg's last words. The interesting line of a failure is the last one;
/// everything above it is the input report, which is long and, when the
/// graph is at fault, irrelevant.
fn tail(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().collect();
    let from = lines.len().saturating_sub(20);
    lines[from..].join("\n")
}
