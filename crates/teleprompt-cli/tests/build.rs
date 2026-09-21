//! `build`, end to end: a script goes in, a video file comes out.
//!
//! This is the acceptance test for the pipeline as a whole — parse,
//! resolve, voice, schedule, publish, render — run against the `null`
//! backend, which costs no model and no network. It needs ffmpeg, and says
//! so rather than passing quietly when there is none.

use std::path::{Path, PathBuf};
use std::process::Command;

use teleprompt_cli::cmd::build::{self, BuildOptions};
use teleprompt_cli::project::Project;

const SCRIPT: &str = "\
---
teleprompt: 1
scene:
  terminal:
    adapter: vhs
---

# A short tour

This paragraph is narrated, and the tape below runs underneath it. {#opening}

```teleprompt scene=terminal
Set TypingSpeed 40ms
Type \"teleprompt plan tour.md\"
Enter
Sleep 1s
```

And a second paragraph, so there is something to follow the first. {#second}

```teleprompt scene=terminal policy=concurrent
Type \"teleprompt build tour.md\"
Enter
```
";

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Seconds of picture, counted frame by frame — a container's own duration
/// reports its longest stream, which hides a video that stops early.
fn picture_seconds(path: &Path) -> f64 {
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
    let rate = fields.next().expect("a frame rate");
    let frames: f64 = fields.next().unwrap().parse().unwrap();
    let (num, den) = rate.split_once('/').expect("a rational frame rate");
    frames * den.parse::<f64>().unwrap() / num.parse::<f64>().unwrap()
}

fn project_with_script(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("tp-build-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let script = dir.join("scripts/tour.md");
    std::fs::write(&script, SCRIPT).unwrap();
    (dir, script)
}

#[tokio::test]
async fn a_script_renders_to_a_video_as_long_as_its_timeline() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project_with_script("renders");
    let project = Project::discover(&dir).unwrap();
    let options = BuildOptions {
        // Small and fast: this test is about the pipeline, not the encoder.
        resolution: Some((320, 180)),
        fps: Some(24),
        ..BuildOptions::defaults(&project, &script, "en")
    };

    let report = build::run_build(&project, &script, "en", &options)
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", build::render_error(&e)));

    assert!(
        report.output.exists(),
        "{} was not written",
        report.output.display()
    );
    let seconds = picture_seconds(&report.output);
    let expected = report.duration_ms as f64 / 1000.0;
    assert!(
        (seconds - expected).abs() < 0.25,
        "the timeline says {expected}s and the video runs {seconds}s"
    );
}

#[tokio::test]
async fn the_report_says_how_much_of_the_picture_is_missing() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project_with_script("slates");
    let project = Project::discover(&dir).unwrap();
    let options = BuildOptions {
        resolution: Some((320, 180)),
        fps: Some(24),
        ..BuildOptions::defaults(&project, &script, "en")
    };

    let report = build::run_build(&project, &script, "en", &options)
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", build::render_error(&e)));

    // Nothing captures scenes yet. A video of correctly-timed slates is a
    // useful artifact and a misleading one to hand over unannounced, so
    // the count is part of the report rather than a footnote.
    assert_eq!(report.slates, 2, "both tape spans rendered as slates");
    assert!(
        report.warnings.iter().any(|w| w.contains("slate")),
        "and the author is told: {:?}",
        report.warnings
    );
}

/// The command as an author types it, and as CI reads it. Driven through
/// the binary because the wiring in `main.rs` — flags, format, exit code —
/// is its own surface, and has been wrong before while the library
/// underneath was right.
#[tokio::test]
async fn the_binary_builds_a_video_and_reports_it_as_json() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project_with_script("binary");
    let out = dir.join("tour.mp4");

    let output = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["--format", "json", "build"])
        .arg(&script)
        .arg("--out")
        .arg(&out)
        .args(["--resolution", "320x180", "--fps", "24"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "build exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("build --format json prints parseable JSON on stdout");
    assert_eq!(json["ok"], true);
    assert_eq!(json["output"], out.display().to_string());
    assert_eq!(json["renderer"], "ffmpeg");
    assert!(json["duration_ms"].as_u64().unwrap() > 0);
    assert_eq!(json["slates"], 2);
    assert!(out.exists());
}

/// `output:` in front matter is the script's own statement about the video
/// it wants. It has been parsed and thrown away since M0; a render is the
/// first thing that can act on it.
#[tokio::test]
async fn front_matter_decides_the_frame_when_no_flag_does() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project_with_script("shape");
    std::fs::write(
        &script,
        SCRIPT.replace(
            "teleprompt: 1\n",
            "teleprompt: 1\noutput:\n  resolution: [480, 270]\n  fps: 12\n",
        ),
    )
    .unwrap();
    let project = Project::discover(&dir).unwrap();

    let report = build::run_build(
        &project,
        &script,
        "en",
        &BuildOptions::defaults(&project, &script, "en"),
    )
    .await
    .unwrap_or_else(|e| panic!("build failed: {}", build::render_error(&e)));

    assert_eq!(frame_shape(&report.output), (480, 270, 12.0));
}

/// Width, height and frame rate, as ffprobe reads them back.
fn frame_shape(path: &Path) -> (u32, u32, f64) {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,avg_frame_rate",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    let csv = String::from_utf8_lossy(&out.stdout);
    let mut fields = csv.trim().split(',');
    let width = fields.next().unwrap().parse().unwrap();
    let height = fields.next().unwrap().parse().unwrap();
    let (num, den) = fields.next().unwrap().split_once('/').unwrap();
    (
        width,
        height,
        num.parse::<f64>().unwrap() / den.parse::<f64>().unwrap(),
    )
}
