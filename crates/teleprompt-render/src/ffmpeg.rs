//! Running ffmpeg, and the audio encode every render ends with.
//!
//! ffmpeg is invoked as a subprocess with an explicit argument vector,
//! never a shell string and never linked. Linking libav would put clang and
//! the libav headers in every build, and a distro ffmpeg built
//! `--enable-gpl` inside an MIT binary. Invoking a subprocess is neither.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

use crate::RenderError;

/// The narration bed's sample rate. Matches what the voice crates emit, so
/// mixing never resamples.
pub(crate) const SAMPLE_RATE: u32 = 24_000;

/// What the finished file's audio track is, which is not what the bed is.
///
/// Mixing runs at whatever the voice backend emitted so nothing resamples
/// on the way through, and that rate has no business deciding what comes
/// out the other end. A render is a file people play — in a browser, on a
/// phone, in whatever preview a review tool embeds — and 48 kHz stereo is
/// what those players are built for. 24 kHz mono is legal AAC and a
/// smaller file, and it is also the shape a player is most free to handle
/// badly: the track is there, the container says so, and nothing comes
/// out. One resample at the encode ends the argument, and costs a few
/// hundred kilobytes.
pub(crate) const DELIVERY_SAMPLE_RATE: u32 = 48_000;

/// Stereo, for the same reason.
pub(crate) const DELIVERY_CHANNELS: u8 = 2;

/// The audio half of an output encode.
pub(crate) fn audio_encode(args: &mut Vec<String>) {
    args.extend([
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "128k".into(),
        "-ar".into(),
        DELIVERY_SAMPLE_RATE.to_string(),
        "-ac".into(),
        DELIVERY_CHANNELS.to_string(),
        "-movflags".into(),
        "+faststart".into(),
    ]);
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
/// Getting this wrong is not a graph bug — it is a hang. ffmpeg is verbose enough on a long script to fill a pipe
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
