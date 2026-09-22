//! Handing a re-timed tape to `vhs` and cutting the beats out of what it
//! draws.
//!
//! The tape teleprompt writes is a pure function and is tested in the
//! crate's own unit tests. What needs a machine is the rest: whether `vhs`
//! records at all here, and whether the beats come back the right length.
//!
//! `vhs` drives `ttyd` through a browser and fails quietly when it cannot
//! — exit 0, no frames, no error has been seen in a container — so the
//! availability probe runs a one-line tape and looks for a file, and these
//! tests skip on what it says. `TELEPROMPT_REQUIRE_VHS` turns that skip
//! into a failure, for a machine that is supposed to have it.

use std::path::{Path, PathBuf};
use std::process::Command;

use teleprompt_capture::{sessions, Beat, CaptureBackend, Frame};
use teleprompt_capture_vhs::render::VhsRender;
use teleprompt_core::Hash;

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
) -> Option<Vec<teleprompt_capture::Shot>> {
    match VhsRender::default().capture(session, frame, dir, &mut |_| {}) {
        Ok(shots) => Some(shots),
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

fn beat(span: &str, source: &str, ms: u64) -> Beat {
    Beat {
        span: span.into(),
        scene: "terminal".into(),
        adapter: "vhs".into(),
        session: None,
        key: Hash::of(span.as_bytes()),
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
/// and the beats are windows onto what came back at the lengths they were
/// scheduled for.
#[test]
fn vhs_renders_the_session_and_the_beats_come_back_their_scheduled_length() {
    let dir = workdir("beats");
    let beats = [
        beat(
            "a#0",
            "Set TypingSpeed 20ms\nType \"echo one\"\nEnter\nSleep 800ms\n",
            1_000,
        ),
        beat(
            "b#0",
            "Set TypingSpeed 20ms\nType \"echo two\"\nEnter\nSleep 800ms\n",
            1_000,
        ),
    ];
    let plan = sessions(&beats, &|_| false);
    assert_eq!(plan.len(), 1, "one scene is one session, so one vhs run");

    let Some(shots) = recorded(
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

    assert_eq!(shots.len(), 2);
    for (shot, b) in shots.iter().zip(&beats) {
        assert_eq!(shot.key, b.key, "filed under the capture key");
        let seconds = seconds_of(&shot.path);
        assert!(
            (seconds - b.duration_ms as f64 / 1000.0).abs() < 0.2,
            "`{}` was scheduled {}ms and came back {seconds}s",
            b.span,
            b.duration_ms
        );
    }
}

/// Nothing of the run is left in the clips directory.
#[test]
fn the_tape_and_the_session_video_do_not_survive() {
    let dir = workdir("tidy");
    let beats = [beat("a#0", "Sleep 400ms\n", 400)];
    let plan = sessions(&beats, &|_| false);
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

/// What a short recording does to the beats past its end.
///
/// `windows` cuts the session video at offsets accumulated from the
/// *scheduled* durations — what the tape ought to take. Nothing compared
/// that against what `vhs` actually produced, so a session that stopped
/// early was cut into clips for frames that were never recorded. ffmpeg
/// writes a 262-byte container with no video stream for those, and the
/// build survived all the way to compose before dying on
///
///     Stream specifier ':v' in filtergraph description … matches no streams
///
/// which is a sentence about stream specifiers for a problem about a
/// truncated recording. Observed on the manual: beats 0-17 exact to
/// within 0.03s, beat 18 cut off mid-way, beats 19-23 empty.
#[test]
fn a_recording_that_stops_early_is_refused_rather_than_cut_into_empty_clips() {
    let dir = workdir("short");
    let video = dir.join("session.mp4");

    // Three beats of one second each; a recording that holds only the
    // first two. `windows` would place the third at 2000..3000ms.
    let session = teleprompt_capture::Session {
        scene: "terminal".into(),
        adapter: "vhs".into(),
        name: Some("short".into()),
        settings: Default::default(),
        steps: vec![
            a_step("a", "Sleep 1s", 1000),
            a_step("b", "Sleep 1s", 1000),
            a_step("c", "Sleep 1s", 1000),
        ],
    };

    assert_eq!(
        super_windows_end(&session),
        3000,
        "the session asks for three seconds of video"
    );

    if !make_silent_video(&video, 2.0) {
        eprintln!("skipping: no ffmpeg to build a fixture with");
        return;
    }

    let why = teleprompt_capture_vhs::render::too_short(&video, 3000)
        .expect("a 2s video cannot satisfy 3s of windows");
    assert!(
        why.contains('c'),
        "the complaint should name the beat that falls off the end: {why}"
    );

    // And the honest case is silent.
    assert!(
        teleprompt_capture_vhs::render::too_short(&video, 2000).is_none(),
        "a recording that covers its windows is not a failure"
    );
}

fn super_windows_end(session: &teleprompt_capture::Session) -> u64 {
    teleprompt_capture_vhs::render::windows(session)
        .last()
        .map(|(from, len)| from + len)
        .unwrap_or(0)
}

fn a_step(span: &str, source: &str, ms: u64) -> teleprompt_capture::Step {
    teleprompt_capture::Step {
        span: span.into(),
        key: Hash::of(span.as_bytes()),
        source: source.into(),
        duration_ms: ms,
        wanted: true,
    }
}

/// A real mp4 of a given length, so the check is exercised against a file
/// ffprobe can actually read rather than against a mock.
fn make_silent_video(at: &Path, seconds: f64) -> bool {
    std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("color=c=black:s=64x64:d={seconds}"))
        .args(["-r", "30", "-pix_fmt", "yuv420p"])
        .arg(at)
        .status()
        .is_ok_and(|s| s.success())
}
