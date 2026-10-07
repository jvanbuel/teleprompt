//! Cutting a session's reel into clips.
//!
//! No recorder here: this is the arithmetic every backend shares, and the
//! check that a recording is long enough to cut to it. It lived in the vhs
//! backend until there were two backends, at which point keeping it there
//! would have meant two copies of the same cut-point arithmetic.

use std::path::Path;

use teleprompt_core::Hash;
use teleprompt_scene::capture::reel::{starved, windows};
use teleprompt_scene::capture::{Session, SessionShot};

/// The windows are the scheduled durations, accumulated.
#[test]
fn the_shots_are_the_scheduled_durations_accumulated() {
    let s = Session {
        scene: "terminal".into(),
        plugin: "vhs".into(),
        name: None,
        settings: Default::default(),
        root: Default::default(),
        shots: vec![
            a_shot("a", "", 800),
            a_shot("b", "", 1_200),
            a_shot("c", "", 400),
        ],
    };
    assert_eq!(windows(&s), vec![(0, 800), (800, 1_200), (2_000, 400)]);
}

/// What a short recording does to the shots past its end.
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
/// truncated recording. Observed on the manual: shots 0-17 exact to
/// within 0.03s, shot 18 cut off mid-way, shots 19-23 empty.
///
/// The line is drawn at a shot that gets nothing, not at a shot that is
/// short. Drawing it at "short" turned 80ms of encoder rounding on a
/// two-second tape into a red build.
#[test]
fn a_recording_that_stops_early_is_refused_rather_than_cut_into_empty_clips() {
    let dir = teleprompt_testkit::test_dir("reel");
    let video = dir.join("session.mp4");

    // Three shots of one second each; a recording holding only two of
    // them. `windows` would place the third at 2000..3000ms.
    let session = Session {
        scene: "terminal".into(),
        plugin: "vhs".into(),
        name: Some("short".into()),
        settings: Default::default(),
        root: Default::default(),
        shots: vec![
            a_shot("a", "Sleep 1s", 1000),
            a_shot("b", "Sleep 1s", 1000),
            a_shot("c", "Sleep 1s", 1000),
        ],
    };

    if !make_silent_video(&video, 2.0) {
        eprintln!("skipping: no ffmpeg to build a fixture with");
        return;
    }

    let why =
        starved(&video, &session).expect("shot `c` begins at 2.00s, which is where the video ends");
    assert!(
        why.contains('c'),
        "the complaint should name the shot that gets nothing: {why}"
    );

    // A recording that merely falls a few frames short of the last shot
    // is not starved: every shot still opens on real footage, and the
    // renderer holds the last frame to fill the slot. This is the case
    // that turned CI red.
    let nearly = dir.join("nearly.mp4");
    assert!(make_silent_video(&nearly, 2.92));
    assert!(
        starved(&nearly, &session).is_none(),
        "80ms short of three seconds is rounding, not a missing shot"
    );

    // And a recording that covers everything is silent.
    let whole = dir.join("whole.mp4");
    assert!(make_silent_video(&whole, 3.0));
    assert!(
        starved(&whole, &session).is_none(),
        "a recording that covers its windows is not a failure"
    );
}

fn a_shot(shot: &str, source: &str, ms: u64) -> SessionShot {
    SessionShot {
        id: shot.into(),
        key: Hash::of(shot.as_bytes()),
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
