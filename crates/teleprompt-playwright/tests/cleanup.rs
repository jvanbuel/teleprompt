//! A capture that fails partway leaves nothing behind among the clips.

use std::collections::BTreeMap;

use teleprompt_playwright::capture::PlaywrightRender;
use teleprompt_scene::capture::{CaptureBackend, Frame, Session, SessionShot};
use teleprompt_scene::core::Hash;

#[test]
fn a_capture_that_cannot_run_leaves_no_work_dir() {
    let out = tempfile::tempdir().unwrap();
    let backend = PlaywrightRender {
        node: "teleprompt-test-no-such-program".into(),
        ..PlaywrightRender::default()
    };
    let session = Session {
        scene: "demo".into(),
        plugin: "playwright".into(),
        name: None,
        settings: BTreeMap::new(),
        root: Default::default(),
        shots: vec![SessionShot {
            id: "a#0".into(),
            key: Hash::of(b"a"),
            source: "await page.goto('about:blank');".into(),
            duration_ms: 1000,
            wanted: true,
        }],
    };
    let frame = Frame {
        width: 640,
        height: 360,
        fps: 30,
    };

    let result = backend.capture(&session, &frame, out.path(), &mut |_| {});
    assert!(result.is_err(), "there is no program to run");
    let left: Vec<_> = std::fs::read_dir(out.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}
