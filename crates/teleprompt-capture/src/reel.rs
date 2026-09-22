//! A session's recording, and the clips cut out of it.
//!
//! Every backend works the same way: run the session once, get one video —
//! the reel — and cut it into a clip per shot. The arithmetic of where
//! those cuts fall, and the check that the reel is long enough to make
//! them, belong to no particular recorder, so they live here rather than
//! once per backend. Two copies of cut-point arithmetic is two videos that
//! agree until they do not.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::Session;

/// Where each shot begins and ends in the session's reel.
///
/// The scheduled durations, accumulated. For a recorder that can be
/// re-timed — a tape — these are the numbers the reel was made from rather
/// than numbers inferred about it afterwards. For one that cannot be, they
/// are what the schedule asked for, and [`starved`] is what checks the
/// recorder delivered it.
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

/// Any shot the reel stops short of entirely.
///
/// `windows` places every shot at an offset accumulated from what the
/// session was *scheduled* to take. That is a prediction about the
/// recorder, and a prediction nothing checks is a bug this project has
/// found more than once: when a recorder stops early — exits 0, writes a
/// valid but truncated video — the shots past its end are cut anyway, and
/// ffmpeg writes a container with no video stream rather than an error.
/// The build then dies much later, in compose, on `Stream specifier ':v' …
/// matches no streams`, which points at the filtergraph rather than at the
/// recording.
///
/// Too short means a shot that gets *nothing* — one whose window begins at
/// or after the last recorded frame. A final shot merely clipped by a few
/// frames is not a failure: ffmpeg returns the footage that exists, the
/// renderer holds the last frame to fill the slot, and the picture is
/// right. Refusing those cost a red build over 80ms of encoder rounding on
/// a two-second tape.
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

/// How long `ffprobe` says a file runs, in milliseconds.
///
/// `None` where it cannot say — no ffprobe, or a file it will not read.
/// A check that cannot run must not invent a failure.
pub fn duration_ms(video: &Path) -> Option<u64> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration"])
        .args(["-of", "default=nw=1:nk=1"])
        .arg(video)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let seconds: f64 = text.trim().parse().ok()?;
    Some((seconds * 1000.0).round() as u64)
}

/// One shot out of the session's reel.
pub fn cut(
    ffmpeg: &str,
    video: &Path,
    clip: &Path,
    from_ms: u64,
    duration_ms: u64,
) -> Result<(), String> {
    let seconds = |ms: u64| format!("{}.{:03}", ms / 1000, ms % 1000);
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
