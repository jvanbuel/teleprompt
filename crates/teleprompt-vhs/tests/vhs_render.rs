//! Handing a re-timed tape to `vhs` and cutting the shots out of what it
//! draws.
//!
//! The tape teleprompt writes is a pure function and is tested in the
//! crate's own unit tests. What needs a machine is the rest: whether `vhs`
//! records at all here, and whether the shots come back the right length.
//!
//! `vhs` drives `ttyd` through a browser and fails quietly when it cannot
//! — exit 0, no frames, no error has been seen in a container — so the
//! availability probe runs a one-line tape and looks for a file, and these
//! tests skip on what it says. `TELEPROMPT_REQUIRE_VHS` turns that skip
//! into a failure, for a machine that is supposed to have it.

use std::path::{Path, PathBuf};
use std::process::Command;

use teleprompt_capture::{sessions, CaptureBackend, Frame, PlannedShot};
use teleprompt_core::Hash;
use teleprompt_vhs::capture::VhsRender;

/// Try the capture, and treat a failure as a skip unless this machine is
/// supposed to manage it.
///
/// Attempting it *is* the check. `vhs` can be installed, have `ttyd`
/// beside it, pass every look-before-you-leap test and still record
/// nothing, because it draws through a browser that may not start. The
/// cheap `unavailable()` deliberately does not try to know that, so this
/// finds out the only way there is.
fn recorded(
    session: &teleprompt_capture::Session,
    frame: &Frame,
    dir: &Path,
) -> Option<Vec<teleprompt_capture::Clip>> {
    match VhsRender::default().capture(session, frame, dir, &mut |_| {}) {
        Ok(clips) => Some(clips),
        Err(why) => {
            assert!(
                std::env::var_os("TELEPROMPT_REQUIRE_VHS").is_none(),
                "TELEPROMPT_REQUIRE_VHS is set and vhs cannot record here: {why}"
            );
            eprintln!("skipping: vhs cannot record here ({why})");
            None
        }
    }
}

fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tp-vhsr-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn shot(shot: &str, source: &str, ms: u64) -> PlannedShot {
    PlannedShot {
        id: shot.into(),
        scene: "terminal".into(),
        adapter: "vhs".into(),
        session: None,
        key: Hash::of(shot.as_bytes()),
        source: source.into(),
        duration_ms: ms,
        settings: Default::default(),
    }
}

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
    let mut f = csv.trim().split(',');
    let rate = f.next().unwrap();
    let frames: f64 = f.next().unwrap().parse().unwrap();
    let (num, den) = rate.split_once('/').unwrap();
    frames * den.parse::<f64>().unwrap() / num.parse::<f64>().unwrap()
}

/// The whole proposition: teleprompt writes the tape, `vhs` renders it,
/// and the shots are windows onto what came back at the lengths they were
/// scheduled for.
#[test]
fn vhs_renders_the_session_and_the_shots_come_back_their_scheduled_length() {
    let dir = workdir("shots");
    let shots = [
        shot(
            "a#0",
            "Set TypingSpeed 20ms\nType \"echo one\"\nEnter\nSleep 800ms\n",
            1_000,
        ),
        shot(
            "b#0",
            "Set TypingSpeed 20ms\nType \"echo two\"\nEnter\nSleep 800ms\n",
            1_000,
        ),
    ];
    let plan = sessions(&shots, &|_| false);
    assert_eq!(plan.len(), 1, "one scene is one session, so one vhs run");

    let Some(clips) = recorded(
        &plan[0],
        &Frame {
            width: 800,
            height: 500,
            fps: 24,
        },
        &dir,
    ) else {
        return;
    };

    assert_eq!(clips.len(), 2);
    for (clip, b) in clips.iter().zip(&shots) {
        assert_eq!(clip.key, b.key, "filed under the capture key");
        let seconds = seconds_of(&clip.path);
        assert!(
            (seconds - b.duration_ms as f64 / 1000.0).abs() < 0.2,
            "`{}` was scheduled {}ms and came back {seconds}s",
            b.id,
            b.duration_ms
        );
    }
}

/// Nothing of the run is left in the clips directory.
#[test]
fn the_tape_and_the_session_video_do_not_survive() {
    let dir = workdir("tidy");
    let shots = [shot("a#0", "Sleep 400ms\n", 400)];
    let plan = sessions(&shots, &|_| false);
    if recorded(
        &plan[0],
        &Frame {
            width: 640,
            height: 400,
            fps: 24,
        },
        &dir,
    )
    .is_none()
    {
        return;
    }

    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.ends_with(".mp4"))
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}
