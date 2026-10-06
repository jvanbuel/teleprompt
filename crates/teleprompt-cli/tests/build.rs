//! `build`, end to end: a script goes in, a video file comes out.
//!
//! This is the acceptance test for the pipeline as a whole — parse,
//! resolve, voice, schedule, publish, render — run against the `null`
//! backend, which costs no model and no network. It needs ffmpeg, and says
//! so rather than passing quietly when there is none.

use std::path::{Path, PathBuf};
use std::process::Command;

use teleprompt::build::Builder;
use teleprompt::project::Project;

const SCRIPT: &str = "\
---
teleprompt: 1
scene:
  terminal:
    plugin: vhs
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
    let present = Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok();
    // A skip that nobody sees is a test that stopped running. CI sets this
    // and installs ffmpeg, so a missing one there is a broken workflow
    // rather than a machine without a renderer.
    assert!(
        present || std::env::var_os("TELEPROMPT_REQUIRE_FFMPEG").is_none(),
        "TELEPROMPT_REQUIRE_FFMPEG is set and there is no ffmpeg on PATH"
    );
    present
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

fn project_with_script(name: &str) -> (teleprompt_testkit::TestDir, PathBuf) {
    let dir = teleprompt_testkit::test_dir(&format!("build-{name}"));
    teleprompt::new::scaffold(&dir).unwrap();
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
    let project = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let opened = project.script(&script, "en");
    // Small and fast: this test is about the pipeline, not the encoder.
    let builder = Builder::new(&opened).resolution(320, 180).fps(24);

    let report = builder
        .build(&mut |_| {})
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));

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
    // Chapters for a YouTube description, beside it too.
    let chapters = std::fs::read_to_string(report.output.with_extension("chapters.txt")).unwrap();
    assert!(chapters.starts_with("0:00 "), "{chapters}");
    // Captions beside the video, named after it, for a player to pick up.
    for ext in ["srt", "vtt"] {
        let path = report.output.with_extension(ext);
        assert!(report.captions.contains(&path), "{:?}", report.captions);
        assert!(std::fs::read_to_string(&path).unwrap().contains("-->"));
    }
}

#[tokio::test]
async fn the_report_says_how_much_of_the_picture_is_missing() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project_with_script("slates");
    let project = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let opened = project.script(&script, "en");
    let builder = Builder::new(&opened).resolution(320, 180).fps(24);

    // A machine with no backend for this scene. Not a contrivance: it is
    // every machine that has no terminal renderer installed, and every
    // scene kind teleprompt can compile and cannot yet run.
    let none = teleprompt_plugin::ScenePlugins::new([]);
    let report = builder
        .plugins(&none)
        .build(&mut |_| {})
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));

    // A video of correctly-timed slates is a useful artifact and a
    // misleading one to hand over unannounced, so the count is part of the
    // report rather than a footnote.
    assert_eq!(report.slates, 2, "both tape shots rendered as slates");
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
    assert_eq!(json["renderer"], "ffmpeg-incremental");
    assert!(json["duration_ms"].as_u64().unwrap() > 0);
    // Not `captured == 2`: this fixture's scenes are `terminal` ones, and
    // whether they record depends on the machine having a working `vhs`.
    // What holds everywhere is that every item is accounted for — one
    // recorded or one held with a slate, never neither and never both.
    let items = json["items"].as_u64().unwrap();
    let captured = json["captured"].as_u64().unwrap();
    let slates = json["slates"].as_u64().unwrap();
    assert_eq!(items, 2);
    assert_eq!(
        captured + slates,
        items,
        "{captured} recorded and {slates} slated does not account for \
         {items} item(s)"
    );
    assert!(out.exists());
}

/// `output:` in front matter is the script's own statement about the video it
/// wants, and a render is what acts on it.
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
    let project = Project::discover(&dir, teleprompt_registry::registry()).unwrap();

    let opened = project.script(&script, "en");
    let report = Builder::new(&opened)
        .build(&mut |_| {})
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));

    // The rate is compared with a tolerance, not for equality. A
    // container reports `avg_frame_rate` as a rational computed from the
    // frames and the duration it actually holds, so a video assembled from
    // captured clips comes back as 12.000321 rather than 12 — a number
    // that is right and is not the same `f64`. Asserting equality here
    // passed for as long as every item was a slate and started failing the
    // day something recorded a real terminal.
    let (width, height, fps) = frame_shape(&report.output);
    assert_eq!((width, height), (480, 270));
    assert!(
        (fps - 12.0).abs() < 0.05,
        "front matter asked for 12fps and the file reports {fps}"
    );
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

/// A render is the one command in teleprompt that takes real time, so it
/// is the one that has to say where it has got to. ffmpeg's own
/// `-progress` stream is the source; nothing scrapes its log.
#[tokio::test]
async fn a_render_reports_how_far_along_it_is() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project_with_script("progress");
    let project = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let opened = project.script(&script, "en");
    let builder = Builder::new(&opened).resolution(320, 180).fps(24);

    let mut seen: Vec<u64> = Vec::new();
    let report = builder
        .build(&mut |p| seen.push(p.rendered_ms))
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));

    assert!(
        seen.len() > 1,
        "progress is reported as it goes, not once at the end: {seen:?}"
    );
    assert!(
        seen.windows(2).all(|w| w[1] >= w[0]),
        "and it never goes backwards: {seen:?}"
    );
    assert_eq!(
        seen.last().copied(),
        Some(report.duration_ms),
        "the last word is the whole length"
    );
}

/// Where a build lands when nobody says. `new` gitignores `build/`, and a
/// render is entirely derived from things that are committed — so it goes
/// there rather than somewhere a checkout will offer to commit it.
#[test]
fn the_default_output_is_inside_the_directory_the_scaffold_ignores() {
    let (dir, script) = project_with_script("defaults");
    let project = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let opened = project.script(&script, "en");

    assert_eq!(
        Builder::new(&opened).output(),
        dir.join("build/tour.en.mp4")
    );
    let ignored = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert!(ignored.contains("build/"), "{ignored}");
}

/// A render is the one expensive step in the pipeline that is entirely
/// derived, so the second build of an unchanged script should not pay for
/// it twice. This is the wiring test — the arithmetic is the render
/// crate's business; what matters here is that `build` hands the renderer
/// a cache at all, and says what it got from it.
#[tokio::test]
async fn a_second_build_of_an_unchanged_script_reuses_the_picture() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project_with_script("reuse");
    let project = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let opened = project.script(&script, "en");
    let builder = Builder::new(&opened).resolution(320, 180).fps(24);

    let cold = builder
        .build(&mut |_| {})
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));
    assert_eq!(cold.renderer, "ffmpeg-incremental");

    let warm = builder
        .build(&mut |_| {})
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));
    assert_eq!(
        warm.reused_ms,
        Some(warm.duration_ms),
        "nothing about the script changed, so nothing needed encoding"
    );
    assert!(picture_seconds(&warm.output) > 0.0);
}

/// And the way out of it. A cache is a claim that two things are the same,
/// and an author who suspects it of being wrong needs a build that makes
/// no such claim — otherwise the only remedy is deleting a directory they
/// have to be told about.
#[tokio::test]
async fn no_cache_re_encodes_everything_and_says_so() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project_with_script("nocache");
    let project = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let opened = project.script(&script, "en");
    let builder = Builder::new(&opened)
        .resolution(320, 180)
        .fps(24)
        .no_cache();

    let report = builder
        .build(&mut |_| {})
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));
    assert_eq!(
        report.renderer, "ffmpeg-incremental",
        "there is one renderer; --no-cache switches off reuse rather than \
         selecting a different one"
    );
    assert_eq!(
        report.reused_ms, None,
        "a render with no cache reused nothing, which is not the same \
         claim as a cold cache reusing none of it"
    );
}

/// The cache must not grow for ever. A build prunes it afterwards rather
/// than leaving it to a command somebody has to remember — a cache that
/// only shrinks when asked still grows without bound, which is the thing
/// a cap is for.
#[tokio::test]
async fn a_build_leaves_the_cache_under_its_cap() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project_with_script("cap");
    let project = Project::discover(&dir, teleprompt_registry::registry()).unwrap();
    let opened = project.script(&script, "en");
    // Nothing at all fits: every entry is evicted on the way out, and
    // the next build is cold. That is exactly what a cap of zero says.
    let builder = Builder::new(&opened)
        .resolution(320, 180)
        .fps(24)
        .cache_max_mb(0);

    let report = builder
        .build(&mut |_| {})
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));
    assert!(report.output.exists(), "the video is still produced");

    let compose = project.caches().compose();
    assert_eq!(
        teleprompt::cache::stats(&compose).bytes,
        0,
        "a cap of nothing kept something"
    );

    // And the real case: a generous cap keeps what the build just made.
    builder
        .cache_max_mb(1_024)
        .build(&mut |_| {})
        .await
        .unwrap_or_else(|e| panic!("build failed: {}", e));
    assert!(
        teleprompt::cache::stats(&compose).bytes > 0,
        "a build under a cap it fits inside threw its own work away"
    );
}

/// `teleprompt cache` is the way to get the disk back without being told
/// which directory to delete.
#[test]
fn the_binary_reports_and_prunes_the_cache() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (dir, script) = project_with_script("cachecmd");
    let built = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .args(["build"])
        .arg(&script)
        .args(["--resolution", "320x180", "--fps", "24"])
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "build exited {}: {}",
        built.status,
        String::from_utf8_lossy(&built.stderr)
    );

    let report = |args: &[&str]| -> serde_json::Value {
        let out = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
            .args(["--format", "json", "cache"])
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "cache exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).expect("cache --format json prints JSON")
    };

    let before = report(&[]);
    assert!(
        before["compose"]["bytes"].as_u64().unwrap() > 0,
        "the build left encoded video behind: {before}"
    );
    assert!(before["pruned"].is_null(), "looking is not pruning");

    let after = report(&["--prune-to-mb", "0"]);
    assert!(after["pruned"]["removed"].as_u64().unwrap() > 0);
    assert_eq!(after["compose"]["bytes"], 0);
}

/// A frame that cannot exist is a mistake in the command, said as one
/// (exit 2), not a failure of the build or a bug to report.
#[test]
fn an_impossible_frame_is_refused_as_a_usage_error() {
    let (_dir, script) = project_with_script("bad-frame");
    for args in [
        ["--fps", "0"],
        ["--resolution", "0x180"],
        ["--resolution", "wide"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_teleprompt"))
            .arg("build")
            .arg(&script)
            .args(args)
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {err}");
        assert!(!err.contains("please report"), "{err}");
    }
}
