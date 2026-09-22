//! Probes shared by the renderer tests.
//!
//! These read a rendered file rather than the argv that asked for it:
//! `adelay` taking the right number is not the same claim as the mix
//! putting the sound there, and only one of the two is what an author
//! hears.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;

/// A fresh working directory for one test, named after it.
pub fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn have_ffmpeg() -> bool {
    let present = Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok();
    // A skip that nobody sees is a test that stopped running. CI sets this
    // and installs ffmpeg, so a missing one there is a broken workflow
    // rather than a machine without a renderer.
    assert!(
        present || std::env::var_os("TELEPROMPT_REQUIRE_FFMPEG").is_none(),
        "TELEPROMPT_REQUIRE_FFMPEG is set and there is no ffmpeg on PATH"
    );
    present
}

/// Seconds of **picture** in `path`, counted frame by frame.
///
/// Not `format=duration`: a container reports the longest stream it holds,
/// so a one-second picture under a three-second audio bed reads back as
/// three seconds and a video that stops early looks correct.
pub fn duration_of(path: &std::path::Path) -> f64 {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames,avg_frame_rate",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    let csv = String::from_utf8_lossy(&out.stdout);
    let mut fields = csv.trim().split(',');
    let rate = fields.next().expect("a frame rate");
    let frames: f64 = fields
        .next()
        .expect("a frame count")
        .parse()
        .expect("a numeric frame count");
    let (num, den) = rate.split_once('/').expect("a rational frame rate");
    frames * den.parse::<f64>().unwrap() / num.parse::<f64>().unwrap()
}

/// A mono 24 kHz WAV of `ms` milliseconds of quiet tone, written by ffmpeg
/// so the fixture does not depend on the voice crates.
pub fn tone(dir: &std::path::Path, name: &str, ms: u64) -> PathBuf {
    let path = dir.join(name);
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-y", "-f", "lavfi", "-t"])
        .arg(format!("{}.{:03}", ms / 1000, ms % 1000))
        .args(["-i", "sine=frequency=220:sample_rate=24000", "-ac", "1"])
        .arg(&path)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    path
}

/// Seconds at which `path` stops being silent, per ffmpeg's own
/// `silencedetect`.
pub fn first_sound(path: &std::path::Path) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-i"])
        .arg(path)
        .args(["-af", "silencedetect=noise=-50dB:d=0.1", "-f", "null", "-"])
        .output()
        .expect("ffmpeg runs");
    let log = String::from_utf8_lossy(&out.stderr);
    log.lines()
        .find_map(|l| l.split("silence_end: ").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no sound at all in the render:\n{log}"))
}

/// Mean luma of the frame at `seconds`, as ffmpeg measures it. A slate is
/// the background colour and reads about 12; anything with a terminal on it
/// reads far higher.
pub fn luma_at(path: &std::path::Path, seconds: f64) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-v", "error", "-ss"])
        .arg(format!("{seconds}"))
        .arg("-i")
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "signalstats,metadata=print:key=lavfi.signalstats.YAVG:file=-",
            "-f",
            "null",
            "-",
        ])
        .output()
        .expect("ffmpeg runs");
    let text =
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    text.split("lavfi.signalstats.YAVG=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no YAVG in ffmpeg's output:\n{text}"))
}

/// A one-second clip of a flat colour, standing in for a capture. The
/// colour is what makes two fixtures different *pictures* rather than two
/// names for the same bytes — which the line cache is entitled to
/// notice, and does.
pub fn colour_clip(dir: &std::path::Path, name: &str, colour: &str) -> PathBuf {
    let path = dir.join(name);
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-y", "-f", "lavfi", "-t", "1", "-i"])
        .arg(format!("color=c={colour}:s=320x180:r=24"))
        .args(["-pix_fmt", "yuv420p"])
        .arg(&path)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    path
}

/// A one-second clip of something bright, standing in for a capture.
pub fn bright_clip(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-y",
            "-f",
            "lavfi",
            "-t",
            "1",
            "-i",
            "color=c=0x808080:s=320x180:r=24",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&path)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    path
}
