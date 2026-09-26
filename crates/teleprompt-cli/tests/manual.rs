//! The manual teleprompt writes about itself.
//!
//! `manual/scripts/cli.md` narrates the command-line interface, and its
//! action blocks are VHS tapes running the commands it describes. Compiling
//! it here is what keeps the claim honest: a manual that lived only in the
//! repository would rot quietly, and one that only a human ever compiled
//! would rot loudly and late.
//!
//! The committed timeline is compiled from a *cold* cache on purpose. Every
//! narration duration in it is a word-count estimate, which is reproducible
//! on any machine with nothing installed; a timeline dubbed from a warm
//! cache would be measured, and CI — which starts with an empty
//! `.teleprompt/cache` every run — could never reproduce it.
//!
//! Which is why these tests copy the manual somewhere else before compiling
//! it. A contributor who has run `teleprompt dub manual/scripts/cli.md`
//! locally has a warm `manual/.teleprompt/cache`, and every narration in it
//! legitimately becomes `measured` — same numbers under the null backend,
//! different provenance, and `diff` says so. Compiling in place would fail
//! this suite on exactly the machines that had exercised the manual most.

use std::path::{Path, PathBuf};
use teleprompt_core::DurationSource;

use teleprompt_cli::cmd::build::{self, BuildOptions};
use teleprompt_cli::cmd::{check::run_check, diff::run_diff, plan::run_plan};
use teleprompt_cli::project::Project;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

/// The manual, in a scratch directory with a guaranteed-cold cache.
fn manual() -> (teleprompt_testkit::TestDir, Project, PathBuf) {
    let src = repo().join("manual");
    let dir = teleprompt_testkit::test_dir("manual");
    for rel in [
        Path::new("teleprompt.toml"),
        Path::new("scripts/cli.md"),
        Path::new("timelines/cli.en.json"),
    ] {
        let to = dir.join(rel);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(src.join(rel), &to)
            .unwrap_or_else(|e| panic!("copying {}: {e}", rel.display()));
    }
    let script = dir.join("scripts/cli.md");
    let project = Project::discover(&dir).expect("the manual is a teleprompt project");
    (dir, project, script)
}

#[test]
fn the_manual_compiles() {
    let (_dir, p, s) = manual();
    let warnings = run_check(&p, &s, "en").expect("the manual must be a valid script");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
}

#[test]
fn the_committed_timeline_is_current() {
    let (_dir, p, s) = manual();
    let d = run_diff(&p, &s, "en").unwrap();
    assert!(
        d.is_empty(),
        "the manual has drifted from `manual/timelines/cli.en.json`; re-run\n  \
         cargo run -- plan manual/scripts/cli.md --format json > manual/timelines/cli.en.json\n\
         {}",
        d.render()
    );
}

#[test]
fn every_terminal_action_is_timed_exactly() {
    // The claim the tape adapter makes and the reason `plan` can report a
    // terminal scene's pacing with no terminal anywhere: a tape states its
    // own timing, so nothing here is a guess.
    let (_dir, p, s) = manual();
    let out = run_plan(&p, &s, "en").unwrap();
    let mut tapes = 0;
    for e in &out.timeline.entries {
        let Some(a) = &e.action else { continue };
        if a.adapter != "vhs" {
            continue;
        }
        tapes += 1;
        assert_eq!(
            a.duration_source,
            DurationSource::Exact,
            "shot {} is {} rather than exact",
            a.shot,
            a.duration_source
        );
    }
    assert!(tapes > 0, "the manual must exercise the tape adapter");
}

#[test]
fn the_manual_demonstrates_every_pacing_policy() {
    // A manual that only ever used the default policy would document the
    // tool it is not.
    let (_dir, p, s) = manual();
    let out = run_plan(&p, &s, "en").unwrap();
    let policies: std::collections::BTreeSet<&str> = out
        .timeline
        .entries
        .iter()
        .map(|e| e.policy.label())
        .collect();
    for expected in ["hold", "concurrent", "fit-action", "trim-action"] {
        assert!(policies.contains(expected), "missing policy {expected}");
    }
}

/// The manual renders. Every stage runs — parse, resolve, voice, schedule,
/// publish, compose — against the `null` backend, which needs no model and
/// no network, and the result is a real file of the length the timeline
/// says it is.
///
/// A pipeline that only ever runs by hand is a pipeline that is broken most
/// of the time, which is why this is here rather than in a script someone
/// remembers to run before a release.
#[tokio::test]
async fn the_manual_renders() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg on PATH");
        return;
    }
    let (_dir, project, script) = manual();
    let options = BuildOptions {
        // Small and slow-framed: CI is checking that the pipeline runs and
        // the arithmetic holds over 25 items, not how a codec looks.
        resolution: Some((320, 180)),
        fps: Some(12),
        ..BuildOptions::defaults(&project, &script, "en")
    };

    let report = build::run_build(&project, &script, "en", &options)
        .await
        .unwrap_or_else(|e| panic!("the manual must render: {}", build::render_error(&e)));

    assert_eq!(report.lines, 21);
    assert_eq!(report.items, 25);

    // `TELEPROMPT_REQUIRE_CAPTURE` is CI saying it has a recorder and
    // expects it to have been used. Nothing read it until now, and the
    // silence was load-bearing: capture is infallible by design, so a
    // tape VHS refuses to parse costs a warning and nothing else, and
    // this test passed on a manual rendered as twenty-five slates. That
    // is how an unparseable tape reached main and stayed there.
    if std::env::var_os("TELEPROMPT_REQUIRE_CAPTURE").is_some() {
        assert_eq!(
            report.slates, 0,
            "TELEPROMPT_REQUIRE_CAPTURE is set and {} of {} item(s) rendered \
             as slates: {:?}",
            report.slates, report.items, report.warnings
        );
    }

    let seconds = picture_seconds(&report.output);
    let expected = report.duration_ms as f64 / 1000.0;
    assert!(
        (seconds - expected).abs() < 0.5,
        "the manual's timeline is {expected}s and its video runs {seconds}s"
    );
}

fn have_ffmpeg() -> bool {
    let present = std::process::Command::new("ffmpeg")
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

/// Seconds of picture, counted frame by frame: a container reports its
/// longest stream, so a video that stops early hides under a full-length
/// audio bed.
fn picture_seconds(path: &Path) -> f64 {
    let out = std::process::Command::new("ffprobe")
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

/// The manual says, of the mark it writes as a comment, that "VHS ignores
/// it, so the file stays a tape VHS itself will run — which is the whole
/// reason to keep a demo in a real tape file rather than in a dialect only
/// teleprompt reads."
///
/// That claim went untested for as long as teleprompt owned a renderer of
/// its own, and it was false. `Type "Type \"teleprompt plan\""` parses
/// under teleprompt and not under VHS, which has no backslash escapes — so
/// the manual demonstrated a dialect only teleprompt reads, in the very
/// paragraph promising it had not invented one.
///
/// `check` cannot catch this: it validates teleprompt's reading of a tape.
/// Only VHS can say what VHS will run, so this asks it.
#[test]
fn every_tape_in_the_manual_is_a_tape_vhs_will_run() {
    use std::io::Write;
    use std::process::Command;

    let vhs_missing = Command::new("vhs")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err();
    if vhs_missing {
        assert!(
            std::env::var_os("TELEPROMPT_REQUIRE_VHS").is_none(),
            "TELEPROMPT_REQUIRE_VHS is set and there is no vhs on PATH"
        );
        eprintln!("skipping: no vhs on PATH");
        return;
    }

    let (_dir, p, s) = manual();
    let (out, _) = teleprompt_cli::cmd::check::compile_script(&p, &s, "en")
        .unwrap_or_else(|e| panic!("the manual compiles: {e:?}"));

    let dir = teleprompt_testkit::test_dir("manual-tapes");

    let mut bad = Vec::new();
    for shot in &out.shots {
        // An `Output` line is the one thing a shot never carries and a
        // tape may not omit; everything after it is the manual's own.
        let path = dir.join(format!("{}.tape", shot.id.replace(['/', '#'], "_")));
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "Output \"{}.mp4\"", path.display()).unwrap();
        f.write_all(shot.source.as_bytes()).unwrap();
        drop(f);

        let said = Command::new("vhs")
            .arg("validate")
            .arg(&path)
            .output()
            .expect("vhs validate runs");
        if !said.status.success() {
            bad.push(format!(
                "  {}: {}",
                shot.id,
                String::from_utf8_lossy(&said.stdout)
                    .lines()
                    .chain(String::from_utf8_lossy(&said.stderr).lines())
                    .filter(|l| l.contains("Invalid") || l.contains("error"))
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        }
    }

    assert!(
        bad.is_empty(),
        "the manual holds {} tape(s) VHS will not run:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
