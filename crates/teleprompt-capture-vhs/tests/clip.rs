//! From a tape to a file the renderer can use.
//!
//! What the pty tests leave out: `agg` draws the recording and ffmpeg
//! containers it, and the result has to be a clip — the right count, the
//! right shape, and not a blank field, which is the exact thing it was
//! brought in to stop being.

use std::path::{Path, PathBuf};
use std::process::Command;

use teleprompt_capture::{sessions, Beat, CaptureBackend, Frame};
use teleprompt_capture_vhs::VhsCapture;
use teleprompt_core::Hash;

fn tools() -> bool {
    let ready = VhsCapture::default().unavailable().is_none();
    assert!(
        ready || std::env::var_os("TELEPROMPT_REQUIRE_CAPTURE").is_none(),
        "TELEPROMPT_REQUIRE_CAPTURE is set and this machine cannot record"
    );
    ready
}

fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tp-clip-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn beat(span: &str, source: &str) -> Beat {
    let mut settings = std::collections::BTreeMap::new();
    settings.insert("columns".to_string(), "80".to_string());
    settings.insert("rows".to_string(), "20".to_string());
    settings.insert("settle_ms".to_string(), "600".to_string());
    Beat {
        span: span.into(),
        scene: "terminal".into(),
        adapter: "vhs".into(),
        session: None,
        key: Hash::of(span.as_bytes()),
        source: source.into(),
        duration_ms: 1_200,
        settings,
    }
}

fn frame() -> Frame {
    Frame {
        width: 480,
        height: 270,
        fps: 12,
    }
}

/// Width, height and frame count of a clip.
fn shape(path: &Path) -> (u32, u32, u64) {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=width,height,nb_read_frames",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    let csv = String::from_utf8_lossy(&out.stdout);
    let mut f = csv.trim().split(',');
    (
        f.next().unwrap().parse().unwrap(),
        f.next().unwrap().parse().unwrap(),
        f.next().unwrap().parse().unwrap(),
    )
}

/// How much ink is on the last frame, which is what a held gap holds.
fn ink(path: &Path) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-v", "error", "-i"])
        .arg(path)
        .args([
            "-vf",
            "signalstats,metadata=print:key=lavfi.signalstats.YMAX:file=-",
            "-f",
            "null",
            "-",
        ])
        .output()
        .expect("ffmpeg runs");
    let text =
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    text.split("lavfi.signalstats.YMAX=")
        .filter_map(|r| r.split_whitespace().next())
        .filter_map(|n| n.parse::<f64>().ok())
        .fold(0.0, f64::max)
}

#[test]
fn a_session_becomes_one_clip_per_wanted_beat() {
    if !tools() {
        eprintln!("skipping: no agg/ffmpeg on PATH");
        return;
    }
    let dir = workdir("clips");
    let beats = [
        beat(
            "a#0",
            "Set TypingSpeed 10ms\nType \"echo one\"\nEnter\nSleep 500ms\n",
        ),
        beat(
            "b#0",
            "Set TypingSpeed 10ms\nType \"echo two\"\nEnter\nSleep 500ms\n",
        ),
    ];
    let plan = sessions(&beats, &|_| false);

    let shots = VhsCapture::default()
        .capture(&plan[0], &frame(), &dir, &mut |_| {})
        .expect("the terminal records");

    assert_eq!(shots.len(), 2);
    for (shot, b) in shots.iter().zip(&beats) {
        assert_eq!(
            shot.path,
            dir.join(format!("{}.mp4", b.key)),
            "a clip is filed under the capture key"
        );
        let (w, h, frames) = shape(&shot.path);
        assert_eq!((w, h), (480, 270), "a clip is the shape the render wants");
        assert!(frames > 0, "`{}` captured no frames at all", b.span);
        assert!(
            ink(&shot.path) > 80.0,
            "`{}` is a blank field, which is the slate it was meant to \
             replace",
            b.span
        );
    }
}

/// Nothing is left behind. A cast and a GIF per beat in the clips
/// directory would be several times the size of the clips themselves.
#[test]
fn the_working_files_do_not_survive_the_capture() {
    if !tools() {
        eprintln!("skipping: no agg/ffmpeg on PATH");
        return;
    }
    let dir = workdir("tidy");
    let beats = [beat("a#0", "Sleep 300ms\n")];
    let plan = sessions(&beats, &|_| false);
    VhsCapture::default()
        .capture(&plan[0], &frame(), &dir, &mut |_| {})
        .expect("records");

    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.ends_with(".mp4"))
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}
