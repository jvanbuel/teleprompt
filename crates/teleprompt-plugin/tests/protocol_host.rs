//! Teleprompt's side of the protocol, against the example plugins in
//! `examples/plugins`: programs written without this crate, in Python, so
//! what they show any language can do.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use teleprompt_core::{BlockId, Hash, ShotId, SourceSpan};
use teleprompt_plugin::capture::{Frame, Session, SessionShot};
use teleprompt_plugin::protocol::host::{self, Found, Plugin};
use teleprompt_plugin::scene::{BlockSource, BodyOrigin, Measured, Validated};

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins")
}

fn runs(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn card() -> Option<Found> {
    if !runs("python3") {
        eprintln!("skipped: no python3 to run the example plugin");
        return None;
    }
    Some(Found {
        name: "card".into(),
        path: examples().join("teleprompt-scene-card"),
    })
}

fn source(body: &str) -> BlockSource {
    BlockSource {
        scene: "card".into(),
        body: body.into(),
        origin: BodyOrigin::Inline {
            fence: SourceSpan {
                line: 10,
                column: 1,
                len: 3,
            },
        },
    }
}

#[test]
fn an_outside_plugin_is_described_and_says_what_it_needs() {
    let Some(found) = card() else { return };
    let plugin = Plugin::new(found);
    let d = plugin.describe().unwrap();
    assert_eq!(d.name, "card");
    assert!(!d.continues);
    let needs = plugin.needs();
    assert_eq!(needs[0].name, "ffmpeg");
    // Asked once: the same tools again, not new ones.
    assert!(std::ptr::eq(needs, plugin.needs()));
}

#[test]
fn its_errors_point_at_the_script_line() {
    let Some(found) = card() else { return };
    let plugin = host::scene(found);
    let errors = plugin
        .scene()
        .validate(&source("color red\ncolour blue\n"))
        .unwrap_err();
    assert_eq!(errors.len(), 1);
    // Body line 1, in a fence on line 10: line 12 of the script.
    assert_eq!(errors[0].span.as_ref().unwrap().line, 12);
    assert!(errors[0].help.as_deref().unwrap().contains("color"));
}

#[test]
fn its_shots_are_numbered_and_timed_as_a_built_in_plugins_are() {
    let Some(found) = card() else { return };
    let plugin = host::scene(found);
    let scene = plugin.scene();
    let v = Validated {
        scene: "card".into(),
        body: "color red\nhold 2s\nmark\ncolor blue\n".into(),
    };
    let shots = scene.shots(&v, &BlockId::new("intro-a")).unwrap();
    assert_eq!(shots.len(), 2);
    assert_eq!(shots[1].id, ShotId::new("intro-a#1"));
    assert_eq!(shots[0].length, Measured::Exact(2000));
    assert_eq!(shots[1].length, Measured::Unknown);
    assert!(!scene.continues());
    let retimed = scene.retime(&shots[1], 1500).unwrap();
    assert!(retimed.ends_with("hold 1500ms"), "{retimed}");
}

#[test]
fn it_captures_a_clip_for_each_wanted_shot() {
    let Some(found) = card() else { return };
    if !runs("ffmpeg") {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let plugin = host::scene(found);
    // Worked out from what it needs, which is here: it was not asked.
    assert_eq!(plugin.capture().unavailable(), None);
    let out = teleprompt_testkit::test_dir("protocol-capture");
    let key = Hash::of(b"blue");
    let session = Session {
        scene: "card".into(),
        plugin: "card".into(),
        name: None,
        settings: Default::default(),
        root: Default::default(),
        shots: vec![SessionShot {
            id: ShotId::new("intro-a#0"),
            key,
            source: "color blue".into(),
            duration_ms: 400,
            wanted: true,
        }],
    };
    let frame = Frame {
        width: 64,
        height: 36,
        fps: 10,
    };
    let mut progress = Vec::new();
    let clips = plugin
        .capture()
        .capture(&session, &frame, &out, &mut |p| progress.push(p))
        .unwrap();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].key, key);
    assert!(clips[0].path.is_file());
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].shot, ShotId::new("intro-a#0"));
}

#[test]
fn a_program_that_is_not_a_plugin_says_so() {
    let dir = teleprompt_testkit::test_dir("protocol-broken");
    let path = dir.join("teleprompt-scene-broken");
    std::fs::write(&path, "#!/bin/sh\necho hello\n").unwrap();
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let plugin = Plugin::new(Found {
        name: "broken".into(),
        path,
    });
    let why = plugin.describe().unwrap_err();
    assert!(why.contains("broken") && why.contains("not JSON"), "{why}");
    assert!(plugin.needs().is_empty());
}
