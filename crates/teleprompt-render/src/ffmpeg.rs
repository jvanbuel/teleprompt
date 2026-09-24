//! ffmpeg as a subprocess with an explicit argument vector, never a shell
//! string and never linked (`docs/design.md#rendering`).

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

use crate::RenderError;

/// The silent bed's rate: Kokoro's, so its lines mix without resampling.
/// Lines at another rate (the null voice emits 48 kHz) are resampled in
/// the mix.
pub(crate) const SAMPLE_RATE: u32 = 24_000;

/// The output track, whatever the bed's rate: 48 kHz stereo is what players
/// handle reliably (`docs/design.md#compose-cache`).
pub(crate) const DELIVERY_SAMPLE_RATE: u32 = 48_000;

pub(crate) const DELIVERY_CHANNELS: u8 = 2;

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

/// Run to completion, reporting output time as it goes. Both pipes must be
/// drained: ffmpeg fills a pipe buffer on a long script, and then hangs.
pub(crate) fn run(
    program: &str,
    args: &[String],
    on_out_time_ms: &mut dyn FnMut(u64),
) -> Result<(), RenderError> {
    // `-progress pipe:1` gives machine-readable lines on stdout; stderr is
    // a log whose format changes between releases.
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

    // Drained on its own thread, or a full stderr pipe deadlocks.
    let mut err = child.stderr.take().expect("stderr was piped");
    let draining = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = err.read_to_string(&mut buf);
        buf
    });

    let out = child.stdout.take().expect("stdout was piped");
    for line in BufReader::new(out).lines().map_while(Result::ok) {
        // Not `out_time_ms`, which is also microseconds despite its name.
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

/// The failure is at the end; above it is the long input report.
fn tail(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().collect();
    let from = lines.len().saturating_sub(20);
    lines[from..].join("\n")
}
