//! A session's recording, and the clips cut out of it.
//!
//! A backend that records a whole session runs it once into one video, the
//! reel, and cuts a clip per shot from it. The cut arithmetic lives here so
//! that no two backends disagree about it.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::Session;

/// Where each shot begins in the reel, and how long it runs: the scheduled
/// durations, accumulated. For a re-timed tape these are what the reel was
/// made from; for other recorders, [`starved`] checks they were delivered.
pub fn windows(session: &Session) -> Vec<(u64, u64)> {
    let mut at = 0;
    session
        .shots
        .iter()
        .map(|shot| {
            let from = at;
            at += shot.duration_ms;
            (from, shot.duration_ms)
        })
        .collect()
}

/// The wanted shots that begin at or after the end of the recording.
///
/// A recorder that stops early can still exit 0, and ffmpeg cuts a shot
/// past the end into a file with no video stream rather than failing; the
/// build then dies in compose, pointing at the filtergraph. A final shot
/// clipped by a few frames is fine: the renderer holds its last frame.
pub fn starved(video: &Path, session: &Session) -> Option<String> {
    let have_ms = duration_ms(video)?;
    let empty: Vec<&str> = session
        .shots
        .iter()
        .zip(windows(session))
        .filter(|(shot, (from_ms, _))| shot.wanted && *from_ms >= have_ms)
        .map(|(shot, _)| shot.id.as_str())
        .collect();
    (!empty.is_empty()).then(|| {
        format!(
            "the recording is {:.2}s and {} shot(s) begin after it ends ({}), \
             so they would be cut from frames that do not exist",
            have_ms as f64 / 1000.0,
            empty.len(),
            empty.join(", "),
        )
    })
}

/// How long `ffprobe` says a file runs, in milliseconds. `None` where it
/// cannot say: a check that cannot run must not invent a failure.
pub fn duration_ms(video: &Path) -> Option<u64> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration"])
        .args(["-of", "default=nw=1:nk=1"])
        .arg(video)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let seconds: f64 = text.trim().parse().ok()?;
    Some(teleprompt_core::time::ms_from_seconds(seconds))
}

/// One shot out of the session's reel.
pub fn cut(
    ffmpeg: &str,
    video: &Path,
    clip: &Path,
    from_ms: u64,
    duration_ms: u64,
) -> Result<(), String> {
    let seconds = teleprompt_core::time::ffmpeg_seconds;
    let status = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
        .arg(seconds(from_ms))
        .arg("-t")
        .arg(seconds(duration_ms))
        .arg("-i")
        .arg(video)
        .args(["-pix_fmt", "yuv420p"])
        .arg(clip)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("{ffmpeg} could not be run: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("{ffmpeg} exited {status}"))
}
