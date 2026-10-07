//! A real app on a real virtual display: skipped where `Xvfb`, `xdotool`,
//! `ffmpeg` or `xterm` is missing.

use std::process::Command;

use teleprompt_desktop::x11::X11Render;
use teleprompt_plugin::capture::{CaptureBackend, Frame, Session, SessionShot};
use teleprompt_plugin::core::Hash;

fn ready() -> bool {
    let render = X11Render::default();
    render.unavailable().is_none() && teleprompt_plugin::core::tool::installed("xterm")
}

fn session(command: &str, shots: &[(&str, u64, bool)]) -> Session {
    Session {
        scene: "term".into(),
        plugin: "x11".into(),
        name: None,
        settings: [
            ("command".to_string(), command.to_string()),
            ("settle_ms".to_string(), "300".to_string()),
        ]
        .into(),
        root: Default::default(),
        shots: shots
            .iter()
            .enumerate()
            .map(|(i, (source, ms, wanted))| SessionShot {
                id: format!("term#{i}").into(),
                key: Hash::of(format!("{i}{source}").as_bytes()),
                source: (*source).to_string(),
                duration_ms: *ms,
                wanted: *wanted,
            })
            .collect(),
    }
}

fn frame() -> Frame {
    Frame {
        width: 640,
        height: 360,
        fps: 12,
    }
}

fn duration_ms(path: &std::path::Path) -> u64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration"])
        .args(["-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .unwrap();
    let s: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    (s * 1000.0).round() as u64
}

/// Each wanted shot is a clip its slot's length; a shot not wanted still
/// runs, for the screen the next opens on, but leaves no clip.
#[test]
fn a_session_runs_the_app_and_cuts_a_clip_per_wanted_shot() {
    if !ready() {
        eprintln!("skipping: needs Xvfb, xdotool, ffmpeg and xterm");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let s = session(
        "xterm -geometry 60x12",
        &[
            ("Type \"echo one\"\nEnter\nSleep 300ms\n", 1_200, false),
            ("Type \"echo two\"\nEnter\n", 1_500, true),
        ],
    );
    let mut progress = Vec::new();
    let clips = X11Render::default()
        .capture(&s, &frame(), dir.path(), &mut |p| progress.push(p.done))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(clips.len(), 1, "only the wanted shot is kept");
    assert_eq!(clips[0].key, s.shots[1].key);
    let ms = duration_ms(&clips[0].path);
    assert!(
        (1_400..=1_600).contains(&ms),
        "the clip is its slot: {ms}ms"
    );
    assert_eq!(progress, [1]);
    if let Ok(keep) = std::env::var("TELEPROMPT_KEEP_CLIP") {
        std::fs::copy(&clips[0].path, keep).unwrap();
    }
    assert!(
        std::fs::read_dir(dir.path()).unwrap().count() == 1,
        "the work directory is gone"
    );
}

#[test]
fn an_app_that_never_opens_a_window_says_so() {
    if !ready() {
        eprintln!("skipping: needs Xvfb, xdotool, ffmpeg and xterm");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let s = session(
        "sh -c 'echo no display for me >&2; exit 3'",
        &[("Enter\n", 500, true)],
    );
    let why = X11Render::default()
        .capture(&s, &frame(), dir.path(), &mut |_| {})
        .unwrap_err()
        .to_string();
    assert!(why.contains("exited"), "{why}");
    assert!(
        why.contains("no display for me"),
        "what the app said: {why}"
    );
}

/// The command runs in the project's directory, as the scene's paths are
/// relative to it.
#[test]
fn the_command_runs_in_the_project() {
    if !ready() {
        eprintln!("skipping: needs Xvfb, xdotool, ffmpeg and xterm");
        return;
    }
    let (dir, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut s = session("sh -c 'pwd >&2; exit 3'", &[("Enter\n", 500, true)]);
    s.root = project.path().canonicalize().unwrap();
    let why = X11Render::default()
        .capture(&s, &frame(), dir.path(), &mut |_| {})
        .unwrap_err()
        .to_string();
    assert!(why.contains(&*s.root.to_string_lossy()), "{why}");
}

#[test]
fn a_scene_without_a_command_says_how_to_name_one() {
    if !ready() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut s = session("", &[("Enter\n", 500, true)]);
    s.settings.remove("command");
    let why = X11Render::default()
        .capture(&s, &frame(), dir.path(), &mut |_| {})
        .unwrap_err()
        .to_string();
    assert!(why.contains("set `command` under [scene.term]"), "{why}");
}
