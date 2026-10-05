//! `build`, with stage 5 in it: an item shows what its scene did.
//!
//! Against the reference scene and the reference capture backend, so the
//! claim under test is the pipeline's rather than any one terminal's: a
//! script whose scenes this build can record produces a video with no
//! slates in it, and a second build of the same script records nothing.

use std::path::{Path, PathBuf};
use std::process::Command;

use teleprompt_cli::cmd::build::{self, BuildOptions};
use teleprompt_cli::project::Project;

const SCRIPT: &str = "\
---
teleprompt: 1
scene:
  demo:
    plugin: mock
---

# A short tour

This paragraph is narrated, and the scene below runs underneath it. {#opening}

```teleprompt scene=demo
wait 800ms
```

And a second paragraph, so there is something to follow the first. {#second}

```teleprompt scene=demo
wait 800ms
```
";

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

fn project(name: &str) -> (teleprompt_testkit::TestDir, PathBuf) {
    let dir = teleprompt_testkit::test_dir(&format!("capture-e2e-{name}"));
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/tour.md");
    std::fs::write(&script, SCRIPT).unwrap();
    (dir, script)
}

fn options(project: &Project, script: &Path) -> BuildOptions {
    BuildOptions {
        resolution: Some((320, 180)),
        fps: Some(24),
        ..BuildOptions::defaults(project, script, "en")
    }
}

/// The whole point of stage 5. Before it, a build was a correctly-paced
/// video of nothing.
#[tokio::test]
async fn a_build_records_its_scenes_and_renders_no_slates() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project("records");
    let p = Project::discover(&dir).unwrap();
    let options = options(&p, &script);

    let report = build::run_build(&p, &script, "en", &options)
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));

    assert_eq!(report.items, 2);
    assert_eq!(report.captured, 2, "both items were recorded");
    assert_eq!(
        report.slates, 0,
        "nothing was left to hold its slot with a blank field: {:?}",
        report.warnings
    );
    assert!(report.output.exists());
}

/// And the second build. Capture is the expensive stage — a tape's sleeps
/// are real seconds and nothing can item realtime — so a script that has
/// not changed must not be recorded again.
#[tokio::test]
async fn a_second_build_records_nothing_and_still_has_no_slates() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project("warm");
    let p = Project::discover(&dir).unwrap();
    let options = options(&p, &script);

    build::run_build(&p, &script, "en", &options)
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));
    let warm = build::run_build(&p, &script, "en", &options)
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));

    assert_eq!(warm.captured, 0, "a warm project re-recorded a scene");
    assert_eq!(warm.slates, 0);
}

/// Editing an item re-records it — and, because a scene is a session,
/// everything after it in that scene. A item downstream of an edit really
/// does show a different screen.
#[tokio::test]
async fn editing_the_first_shot_re_records_the_second() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project("edit");
    let p = Project::discover(&dir).unwrap();
    let options = options(&p, &script);

    build::run_build(&p, &script, "en", &options)
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));

    std::fs::write(&script, SCRIPT.replacen("wait 800ms", "wait 900ms", 1)).unwrap();
    let after = build::run_build(&p, &script, "en", &options)
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));

    assert_eq!(
        after.captured, 2,
        "the edited item and the one that opens on the screen it leaves"
    );
    assert_eq!(after.slates, 0);
}

/// A scene nothing here can record is a warning and a slate, not a failed
/// build. The timing is still real, and a video with a hole in it is more
/// use than no video.
#[tokio::test]
async fn a_scene_with_no_backend_is_a_slate_and_says_why() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project("unbacked");
    let p = Project::discover(&dir).unwrap();
    let options = options(&p, &script);

    // A machine with no backend for this scene — which is every machine
    // that has not installed one, and every scene kind teleprompt can
    // compile and cannot yet run.
    let report = build::run_build_with_capture(
        &build::renderer(&options),
        &teleprompt_plugin::ScenePlugins::new([]),
        &p,
        &script,
        "en",
        &options,
        &mut |_| {},
    )
    .await
    .unwrap_or_else(|e| panic!("build failed: {}", e));

    assert_eq!(report.captured, 0);
    assert_eq!(report.slates, 2);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("demo") && w.contains("mock")),
        "the warning names the scene the author wrote and the plugin it \
         needs: {:?}",
        report.warnings
    );
}
