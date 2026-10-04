//! The reference backend, against real ffmpeg.
//!
//! What it draws is not the interesting part. What it has to get right is
//! everything a real backend has to get right: keep the wanted shots and
//! only those, file each clip under its key, and make the slot.

use std::path::Path;
use std::process::Command;

use teleprompt_core::Hash;
use teleprompt_plugin::capture::mock::MockCapture;
use teleprompt_plugin::capture::{sessions, CaptureBackend, Frame, PlannedShot};

fn have_ffmpeg() -> bool {
    let present = Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok();
    assert!(
        present || std::env::var_os("TELEPROMPT_REQUIRE_FFMPEG").is_none(),
        "TELEPROMPT_REQUIRE_FFMPEG is set and there is no ffmpeg on PATH"
    );
    present
}

fn workdir(name: &str) -> teleprompt_testkit::TestDir {
    teleprompt_testkit::test_dir(&format!("capture-{name}"))
}

fn shot(shot: &str, ms: u64) -> PlannedShot {
    PlannedShot {
        id: shot.into(),
        scene: "terminal".into(),
        plugin: "mock".into(),
        session: None,
        key: Hash::of(shot.as_bytes()),
        source: "wait 1000ms".into(),
        duration_ms: ms,
        settings: Default::default(),
    }
}

/// Seconds of picture, counted frame by frame.
fn seconds_of(path: &Path) -> f64 {
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
    let rate = fields.next().unwrap();
    let frames: f64 = fields.next().unwrap().parse().unwrap();
    let (num, den) = rate.split_once('/').unwrap();
    frames * den.parse::<f64>().unwrap() / num.parse::<f64>().unwrap()
}

#[test]
fn a_session_yields_one_clip_per_wanted_step_named_by_its_key() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("mock-clips");
    let shots = [shot("a#0", 1_000), shot("b#0", 2_000)];
    let plan = sessions(&shots, &|_| false);

    let clips = MockCapture::default()
        .capture(
            &plan[0],
            &Frame {
                width: 320,
                height: 180,
                fps: 24,
            },
            &dir,
            &mut |_| {},
        )
        .expect("the reference backend captures");

    assert_eq!(clips.len(), 2);
    for (clip, b) in clips.iter().zip(&shots) {
        assert_eq!(clip.key, b.key);
        assert_eq!(
            clip.path,
            dir.join(format!("{}.mp4", b.key)),
            "a clip is filed under the capture key the manifest published"
        );
        let seconds = seconds_of(&clip.path);
        assert!(
            (seconds - b.duration_ms as f64 / 1000.0).abs() < 0.1,
            "`{}` was scheduled {}ms and captured {seconds}s",
            b.id,
            b.duration_ms
        );
    }
}

/// A shot whose clip is in hand is run through and not kept. Writing it
/// again would be work the whole cache exists to avoid.
#[test]
fn a_step_nothing_wants_produces_no_clip() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("mock-skip");
    let shots = [shot("a#0", 1_000), shot("b#0", 1_000)];
    let already = shots[0].key;
    let plan = sessions(&shots, &|k| *k == already);

    let clips = MockCapture::default()
        .capture(
            &plan[0],
            &Frame {
                width: 160,
                height: 90,
                fps: 24,
            },
            &dir,
            &mut |_| {},
        )
        .expect("captures");

    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].key, shots[1].key);
    assert!(!dir.join(format!("{already}.mp4")).exists());
}

/// Two shots are two pictures. A backend that drew the same frame for both
/// would pass every timing assertion and produce a video nobody can follow.
#[test]
fn two_steps_do_not_look_the_same() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let dir = workdir("mock-distinct");
    let shots = [shot("a#0", 500), shot("b#0", 500)];
    let plan = sessions(&shots, &|_| false);
    let clips = MockCapture::default()
        .capture(
            &plan[0],
            &Frame {
                width: 160,
                height: 90,
                fps: 24,
            },
            &dir,
            &mut |_| {},
        )
        .expect("captures");

    let luma = |path: &Path| -> f64 {
        let out = Command::new("ffmpeg")
            .args(["-hide_banner", "-v", "error", "-i"])
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
        let text = String::from_utf8_lossy(&out.stderr).into_owned()
            + &String::from_utf8_lossy(&out.stdout);
        text.split("lavfi.signalstats.YAVG=")
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("no YAVG:\n{text}"))
    };

    let (a, b) = (luma(&clips[0].path), luma(&clips[1].path));
    assert!((a - b).abs() > 1.0, "both clips read {a} and {b}");
    assert!(a > 40.0 && b > 40.0, "a clip that renders black is a slate");
}
