//! Letting `vhs` do the rendering.
//!
//! This is the obvious way round and it should have been first: teleprompt
//! already re-times every tape so a span lasts exactly as long as the
//! sentence over it, so the tape handed to `vhs` *is* the schedule. Run it,
//! and the video's timeline is the timeline — there is nothing to
//! reimplement and nothing to estimate.
//!
//! One run per session, because a scene is a session: the spans are
//! concatenated in order so the program stays running across beats, and
//! the beats are then windows onto the one video. Their boundaries are the
//! scheduled durations — the same numbers the tape was written from — so
//! they are right by construction rather than by inference.
//!
//! What this does not fix is that `vhs` needs `ttyd` and a browser. Where
//! it is not there, [`crate::VhsCapture`] drives a pty instead.

use std::path::Path;
use std::process::{Command, Stdio};

use teleprompt_capture::{CaptureBackend, CaptureError, Frame, Progress, Session, Shot};

/// The tape `vhs` should run for a whole session.
///
/// `output` is where `vhs` writes the one video the beats are cut from.
pub fn tape_for(session: &Session, frame: &Frame, output: &str) -> String {
    let mut out = String::new();
    // Quoted: VHS's parser reads a bare `/` as the start of a command, so
    // an unquoted absolute path is three syntax errors and no recording.
    out.push_str(&format!("Output \"{output}\"\n"));

    // VHS sizes its window in pixels, which is what the render wants the
    // picture to be. The scene's `columns`/`rows` are the pty backend's
    // unit and are deliberately not used here: two backends, each told the
    // size in the unit it actually takes.
    out.push_str(&format!("Set Width {}\n", frame.width));
    out.push_str(&format!("Set Height {}\n", frame.height));
    out.push_str(&format!("Set Framerate {}\n", frame.fps));
    if let Some(theme) = session.settings.get("theme") {
        out.push_str(&format!("Set Theme \"{theme}\"\n"));
    }
    if let Some(size) = session.settings.get("font_size") {
        out.push_str(&format!("Set FontSize {size}\n"));
    }

    // The shell drawing its first prompt is not part of any beat, and a
    // video that opens on it would put every beat late by however long it
    // took. `Hide` is what VHS has for exactly this.
    out.push_str("Hide\n");
    for (key, value) in session.nested("env") {
        // `Env` is refused in an authored tape — the environment belongs to
        // the scene — which is precisely why it is set here, from the
        // scene, in the one tape teleprompt writes itself.
        //
        // Mind what `Env PATH` does. VHS applies `Env` to its *own*
        // process, not only to the shell it records, and VHS finds the
        // browser it screenshots through by searching PATH. So a scene
        // that sets PATH so its terminal can find the program being
        // demonstrated also decides which Chromium VHS launches — and if
        // that PATH has no usable browser on it, capture fails talking
        // about a debug URL rather than about a path. A scene's PATH
        // wants a browser on it as well as the program.
        out.push_str(&format!("Env {key} \"{value}\"\n"));
    }
    out.push_str(&format!(
        "Sleep {}ms\n",
        session.number("settle_ms", 800).max(1)
    ));
    out.push_str("Show\n");

    for step in &session.steps {
        out.push_str(step.source.trim_end());
        out.push('\n');
    }
    out
}

/// Where each beat begins and ends in the session's video.
///
/// The scheduled durations, accumulated. Not a guess: the tape was
/// re-written to last exactly this long, so these are the numbers the video
/// was made from rather than numbers inferred about it afterwards.
pub fn windows(session: &Session) -> Vec<(u64, u64)> {
    let mut at = 0;
    session
        .steps
        .iter()
        .map(|step| {
            let from = at;
            at += step.duration_ms;
            (from, step.duration_ms)
        })
        .collect()
}

/// Records `terminal` scenes by handing a re-timed tape to `vhs`.
///
/// The preferred backend where `vhs` will run. It is not a wrapper around
/// a renderer teleprompt also has — it is the renderer, and teleprompt's
/// only job is to write the tape that produces the schedule.
#[derive(Debug, Clone)]
pub struct VhsRender {
    pub vhs: String,
    pub ffmpeg: String,
}

impl Default for VhsRender {
    fn default() -> Self {
        Self {
            vhs: "vhs".into(),
            ffmpeg: "ffmpeg".into(),
        }
    }
}

impl CaptureBackend for VhsRender {
    fn id(&self) -> &'static str {
        "vhs"
    }

    fn adapter(&self) -> &'static str {
        "vhs"
    }

    fn unavailable(&self) -> Option<String> {
        // Cheap, because this is asked to *choose* a backend — on every
        // capture and on every `doctor`. Running a probe recording here
        // was accurate and cost a minute, which made `doctor` useless.
        //
        // It is also not the whole question: `vhs` can be installed and
        // still record nothing, quietly. That case is caught where it
        // happens, in `capture`, and the caller falls back to the next
        // backend — which is a better answer than a slow guess made in
        // advance.
        let missing: Vec<&str> = [self.vhs.as_str(), "ttyd", self.ffmpeg.as_str()]
            .into_iter()
            .filter(|p| !crate::on_path(p))
            .collect();
        (!missing.is_empty()).then(|| format!("{} is not on PATH", missing.join(" and ")))
    }

    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Shot>, CaptureError> {
        std::fs::create_dir_all(out_dir).map_err(|source| CaptureError::Io {
            path: out_dir.display().to_string(),
            source,
        })?;
        let work = out_dir.join(format!(".vhs-{}", std::process::id()));
        std::fs::create_dir_all(&work).map_err(|source| CaptureError::Io {
            path: work.display().to_string(),
            source,
        })?;

        let failed = |span: &str, reason: String| CaptureError::Failed {
            backend: "vhs".to_string(),
            span: span.to_string(),
            reason,
        };
        let first = session
            .steps
            .first()
            .map(|s| s.span.clone())
            .unwrap_or_default();

        let video = work.join("session.mp4");
        let tape = work.join("session.tape");
        std::fs::write(
            &tape,
            tape_for(session, frame, &video.display().to_string()),
        )
        .map_err(|source| CaptureError::Io {
            path: tape.display().to_string(),
            source,
        })?;

        // stderr is kept rather than inherited, because the failure worth
        // reporting is the one with nothing in it — and when there *is*
        // something, it is the only clue there will be.
        let ran = Command::new(&self.vhs)
            .arg(&tape)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| failed(&first, format!("{} could not be run: {e}", self.vhs)))?;
        let said = |ran: &std::process::Output| {
            let text = String::from_utf8_lossy(&ran.stderr).into_owned()
                + &String::from_utf8_lossy(&ran.stdout);
            let tail: Vec<&str> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            let from = tail.len().saturating_sub(4);
            tail[from..].join(" / ")
        };
        if !ran.status.success() {
            return Err(failed(
                &first,
                format!("{} exited {}: {}", self.vhs, ran.status, said(&ran)),
            ));
        }
        // The failure that has no error in it: exit 0, no file. Seen in a
        // container and on a stock CI runner, both times after `vhs`
        // printed its usual "Creating …" and its usual closing advert.
        if !video.metadata().is_ok_and(|m| m.len() > 0) {
            return Err(failed(
                &first,
                format!(
                    "{} exited 0 and wrote no video to {}; it said: {}",
                    self.vhs,
                    video.display(),
                    said(&ran)
                ),
            ));
        }

        // The windows are a prediction about how long the tape would take
        // to run. Check it against what was recorded before cutting to it.
        let needs_ms = windows(session)
            .last()
            .map(|(from, len)| from + len)
            .unwrap_or(0);
        if let Some(why) = too_short(&video, needs_ms) {
            let late = session
                .steps
                .iter()
                .zip(windows(session))
                .find(|(_, (from, _))| duration_ms(&video).is_some_and(|have| *from >= have))
                .map(|(step, _)| step.span.clone())
                .unwrap_or_else(|| first.clone());
            return Err(failed(&late, why));
        }

        let wanted = session.wanted();
        let mut shots = Vec::new();
        for (step, (from_ms, duration_ms)) in session.steps.iter().zip(windows(session)) {
            if !step.wanted {
                continue;
            }
            let clip = out_dir.join(format!("{}.mp4", step.key));
            cut(&self.ffmpeg, &video, &clip, from_ms, duration_ms)
                .map_err(|reason| failed(&step.span, reason))?;
            shots.push(Shot {
                key: step.key,
                path: clip,
            });
            on_progress(Progress {
                scene: session.scene.clone(),
                span: step.span.clone(),
                done: shots.len(),
                of: wanted,
            });
        }

        let _ = std::fs::remove_dir_all(&work);
        Ok(shots)
    }
}

/// Whether the recording is too short for the windows cut from it.
///
/// `windows` places every beat at an offset accumulated from what the
/// tape was *scheduled* to take. That is a prediction about the recorder,
/// and a prediction nothing checks is the bug this crate keeps finding:
/// when `vhs` stops early — exits 0, writes a valid but truncated video —
/// the beats past its end are cut anyway, and ffmpeg emits a container
/// with no video stream rather than an error. The build then dies much
/// later, in compose, saying
///
/// ```text
/// Stream specifier ':v' in filtergraph description … matches no streams
/// ```
///
/// which points at the filtergraph rather than at the recording.
///
/// A frame of tolerance, because the last window ends on a boundary the
/// encoder rounds.
pub fn too_short(video: &Path, needs_ms: u64) -> Option<String> {
    let have_ms = duration_ms(video)?;
    (have_ms + 40 < needs_ms).then(|| {
        format!(
            "the recording is {:.2}s but its beats need {:.2}s; \
             {:.2}s of it was never recorded, so the last beat(s) would be \
             cut from frames that do not exist",
            have_ms as f64 / 1000.0,
            needs_ms as f64 / 1000.0,
            (needs_ms - have_ms) as f64 / 1000.0,
        )
    })
}

/// How long `ffprobe` says a file runs, in milliseconds.
///
/// `None` where it cannot say — no ffprobe, or a file it will not read.
/// A check that cannot run must not invent a failure.
fn duration_ms(video: &Path) -> Option<u64> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration"])
        .args(["-of", "default=nw=1:nk=1"])
        .arg(video)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let seconds: f64 = text.trim().parse().ok()?;
    Some((seconds * 1000.0).round() as u64)
}

/// One beat out of the session's video.
fn cut(
    ffmpeg: &str,
    video: &Path,
    clip: &Path,
    from_ms: u64,
    duration_ms: u64,
) -> Result<(), String> {
    let seconds = |ms: u64| format!("{}.{:03}", ms / 1000, ms % 1000);
    let status = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
        .arg(seconds(from_ms))
        .arg("-t")
        .arg(seconds(duration_ms))
        .arg("-i")
        .arg(video)
        .args(["-pix_fmt", "yuv420p"])
        .arg(clip)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("{ffmpeg} could not be run: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("{ffmpeg} exited {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use teleprompt_capture::Step;
    use teleprompt_core::Hash;

    fn session(steps: &[(&str, u64)]) -> Session {
        Session {
            scene: "terminal".into(),
            adapter: "vhs".into(),
            name: None,
            settings: Default::default(),
            steps: steps
                .iter()
                .map(|(source, ms)| Step {
                    span: "s".into(),
                    key: Hash::of(source.as_bytes()),
                    source: (*source).to_string(),
                    duration_ms: *ms,
                    wanted: true,
                })
                .collect(),
        }
    }

    fn frame() -> Frame {
        Frame {
            width: 1200,
            height: 600,
            fps: 24,
        }
    }

    /// The spans go in in order and unedited. They have already been
    /// re-timed to their slots; rewriting them here would be a second
    /// opinion about a number that is already settled.
    #[test]
    fn a_session_is_one_tape_of_its_spans_in_order() {
        let tape = tape_for(
            &session(&[("Type \"one\"\n", 500), ("Type \"two\"\n", 500)]),
            &frame(),
            "/out/session.mp4",
        );
        let one = tape.find("Type \"one\"").expect("the first span is in it");
        let two = tape.find("Type \"two\"").expect("and the second");
        assert!(one < two, "in order:\n{tape}");
        assert!(
            tape.starts_with("Output \"/out/session.mp4\"\n"),
            "an unquoted path is three syntax errors to VHS's parser:\n{tape}"
        );
    }

    /// The shell's first prompt belongs to no beat. Recording it would put
    /// every beat late by however long it took to draw.
    #[test]
    fn the_shell_starting_up_is_hidden() {
        let tape = tape_for(&session(&[("Type \"x\"\n", 100)]), &frame(), "o.mp4");
        let hide = tape.find("Hide\n").expect("a hidden prelude");
        let show = tape.find("Show\n").expect("that ends");
        let first = tape.find("Type \"x\"").unwrap();
        assert!(hide < show && show < first, "{tape}");
    }

    /// The scene's environment goes in the tape teleprompt writes, which is
    /// the one place `Env` is allowed — an authored tape is refused it
    /// because a scene's blocks share a shell settled before the first of
    /// them runs.
    #[test]
    fn the_scene_environment_is_set_in_the_hidden_prelude() {
        let mut s = session(&[("Type \"x\"\n", 100)]);
        s.settings
            .insert("env.PATH".into(), "target/release".into());
        let tape = tape_for(&s, &frame(), "o.mp4");
        let env = tape.find("Env PATH \"target/release\"").expect("set");
        assert!(env < tape.find("Show\n").unwrap(), "before the recording");
    }

    /// The beats are windows onto the one video, at the offsets the tape
    /// was written to produce.
    #[test]
    fn the_beats_are_the_scheduled_durations_accumulated() {
        let windows = windows(&session(&[("a", 800), ("b", 1_200), ("c", 400)]));
        assert_eq!(windows, vec![(0, 800), (800, 1_200), (2_000, 400)]);
    }
}
