//! Capturing and building from the app: `teleprompt capture` and `build`,
//! their progress read as they go, and the video they made.

use teleprompt_gtk::make::{built, event, run, Job, Progress};

#[test]
fn each_stage_s_progress_is_read_from_its_event() {
    let voice = event(r#"{"event":"progress","stage":"voice","done":1,"of":2,"line":"welcome"}"#);
    assert_eq!(
        voice,
        Some(Progress {
            stage: "voice".into(),
            done: 1,
            of: 2,
            what: "welcome".into()
        })
    );
    let shot = event(r#"{"event":"progress","stage":"capture","done":2,"of":3,"scene":"vhs","shot":"deploy-a#0"}"#)
        .unwrap();
    assert_eq!(
        (shot.done, shot.of, shot.what.as_str()),
        (2, 3, "deploy-a#0")
    );
    let render =
        event(r#"{"event":"progress","stage":"render","done_ms":4500,"of_ms":9000}"#).unwrap();
    assert_eq!((render.done, render.of), (4500, 9000));
}

#[test]
fn anything_else_on_stderr_is_not_progress() {
    assert_eq!(event("warning: 2 narration durations are estimated"), None);
    assert_eq!(event(r#"{"event":"listening"}"#), None);
}

/// What the app says while it works.
#[test]
fn progress_reads_as_what_is_being_done() {
    let p = |stage: &str, done, of, what: &str| Progress {
        stage: stage.into(),
        done,
        of,
        what: what.into(),
    };
    assert_eq!(
        p("voice", 1, 2, "welcome").label(),
        "Voicing lines · 1 of 2"
    );
    assert_eq!(
        p("capture", 2, 3, "deploy-a#0").label(),
        "Capturing deploy-a · 2 of 3"
    );
    assert_eq!(p("render", 4500, 9000, "").label(), "Rendering · 50%");
    assert!((p("render", 4500, 9000, "").fraction() - 0.5).abs() < 1e-9);
}

#[test]
fn a_build_names_the_video_it_made() {
    let report = r#"{"ok": true, "output": "/p/build/tour.en.mp4", "duration_ms": 9000}"#;
    assert_eq!(
        built(report).unwrap(),
        std::path::PathBuf::from("/p/build/tour.en.mp4")
    );
    assert!(built(r#"{"ok": false, "errors": ["ffmpeg exited"]}"#)
        .unwrap_err()
        .contains("ffmpeg exited"));
}

/// Against the real `teleprompt`: capture, then build, saying how far each
/// got. Skipped without `TELEPROMPT_BIN`.
#[test]
fn building_captures_then_renders_the_video() {
    let Some(binary) = std::env::var_os("TELEPROMPT_BIN").map(std::path::PathBuf::from) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("p");
    assert!(std::process::Command::new(&binary)
        .arg("new")
        .arg(&project)
        .status()
        .unwrap()
        .success());
    let script = project.join("scripts/demo.md");
    let mut seen: Vec<String> = Vec::new();
    let video = run(&binary, &script, Job::Build, |p| seen.push(p.stage)).unwrap();
    assert!(video.unwrap().is_file());
    assert!(seen.contains(&"capture".to_string()), "{seen:?}");
    assert!(seen.contains(&"render".to_string()), "{seen:?}");
    // Capturing alone makes no video.
    assert_eq!(run(&binary, &script, Job::Capture, |_| {}).unwrap(), None);
}
