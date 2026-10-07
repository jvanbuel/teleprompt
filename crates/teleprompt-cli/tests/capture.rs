//! `build`, with stage 5 in it: an item shows what its scene did.
//!
//! Against the reference scene and the reference capture backend, so the
//! claim under test is the pipeline's rather than any one terminal's: a
//! script whose scenes this build can record produces a video with no
//! slates in it, and a second build of the same script records nothing.

use std::path::{Path, PathBuf};
use std::process::Command;

use teleprompt_project::build::Builder;
use teleprompt_project::project::Project;
use teleprompt_project::project::Script;

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
    teleprompt_cli::commands::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/tour.md");
    std::fs::write(&script, SCRIPT).unwrap();
    (dir, script)
}

fn builder(script: &Script) -> Builder<'_> {
    Builder::new(script).resolution(320, 180).fps(24)
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
    let p = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let opened = p.script(&script, "en");
    let builder = builder(&opened);

    let report = builder
        .build(&teleprompt_core::Silent)
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
    let p = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let opened = p.script(&script, "en");
    let builder = builder(&opened);

    builder
        .build(&teleprompt_core::Silent)
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));
    let warm = builder
        .build(&teleprompt_core::Silent)
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
    let p = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let opened = p.script(&script, "en");
    let builder = builder(&opened);

    builder
        .build(&teleprompt_core::Silent)
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));

    std::fs::write(&script, SCRIPT.replacen("wait 800ms", "wait 900ms", 1)).unwrap();
    let after = builder
        .build(&teleprompt_core::Silent)
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
    let p = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let opened = p.script(&script, "en");
    let builder = builder(&opened);

    // A machine with no backend for this scene — which is every machine
    // that has not installed one, and every scene kind teleprompt can
    // compile and cannot yet run.
    let none = teleprompt_scene::ScenePlugins::new([]);
    let report = builder
        .plugins(&none)
        .build(&teleprompt_core::Silent)
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

fn teleprompt(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

/// `dub --clips` makes the narration directory a whole package: each shot's
/// clip beside the manifest, under the key the manifest names, recorded on
/// the way if it was missing. The manifest is the one `dub` writes alone.
#[test]
fn dub_with_clips_puts_each_shot_beside_the_manifest() {
    if !have_ffmpeg() {
        return;
    }
    let (dir, _) = project("dub-clips");
    let plain = teleprompt(&dir, &["dub", "scripts/tour.md", "--out", "plain"]);
    assert!(
        plain.status.success(),
        "{}",
        String::from_utf8_lossy(&plain.stderr)
    );
    let out = teleprompt(
        &dir,
        &["dub", "scripts/tour.md", "--out", "package", "--clips"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let manifest = std::fs::read_to_string(dir.join("package/en/narration.json")).unwrap();
    assert_eq!(
        manifest,
        std::fs::read_to_string(dir.join("plain/en/narration.json")).unwrap(),
        "--clips changes nothing in the manifest"
    );
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    let keys: Vec<String> = manifest["shots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["capture_key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(keys.len(), 2);
    let clips = dir.join("package/en/clips");
    for key in &keys {
        let clip = clips.join(format!("{key}.mp4"));
        assert!(
            std::fs::metadata(&clip).unwrap().len() > 0,
            "{}",
            clip.display()
        );
    }
    assert!(
        !dir.join("plain/en/clips").exists(),
        "no clips without --clips"
    );
}

/// A clip no shot names any more is taken out, so the package holds what
/// the manifest describes and nothing else.
#[test]
fn dub_with_clips_drops_clips_no_shot_names() {
    if !have_ffmpeg() {
        return;
    }
    let (dir, _) = project("dub-clips-stale");
    let stale = dir.join("package/en/clips/0000.mp4");
    std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
    std::fs::write(&stale, b"old").unwrap();
    let out = teleprompt(
        &dir,
        &["dub", "scripts/tour.md", "--out", "package", "--clips"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!stale.exists());
    assert_eq!(
        std::fs::read_dir(dir.join("package/en/clips"))
            .unwrap()
            .count(),
        2
    );
}

#[test]
fn dub_clips_and_check_do_not_go_together() {
    let (dir, _) = project("dub-clips-check");
    let out = teleprompt(
        &dir,
        &["dub", "scripts/tour.md", "--out", "o", "--clips", "--check"],
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
