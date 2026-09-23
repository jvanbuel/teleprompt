//! Letting `vhs` do the rendering.
//!
//! This is the obvious way round and it should have been first: teleprompt
//! already re-times every tape so a shot lasts exactly as long as the
//! sentence over it, so the tape handed to `vhs` *is* the schedule. Run it,
//! and the video's timeline is the timeline — there is nothing to
//! reimplement and nothing to estimate.
//!
//! One run per session, because a scene is a session: the shots are
//! concatenated in order so the program stays running across shots, and
//! the shots are then windows onto the one video. Their boundaries are the
//! scheduled durations — the same numbers the tape was written from — so
//! they are right by construction rather than by inference.
//!
//! What this does not fix is that `vhs` needs `ttyd` and a browser. Where
//! it is not there, [`crate::VhsCapture`] drives a pty instead.

use std::path::Path;
use std::process::{Command, Stdio};

use teleprompt_capture::reel::{cut, starved, windows};
use teleprompt_capture::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session, WorkDir};

/// The tape `vhs` should run for a whole session.
///
/// `output` is where `vhs` writes the one video the shots are cut from.
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

    // The shell drawing its first prompt is not part of any shot, and a
    // video that opens on it would put every shot late by however long it
    // took. `Hide` is what VHS has for exactly this.
    out.push_str("Hide\n");
    for (key, value) in session.nested("env") {
        // PATH is the one variable a scene must not be allowed to clear:
        // see `path_with_fallback`.
        if key == "PATH" {
            let inherited = std::env::var("PATH").ok();
            out.push_str(&format!(
                "Env PATH \"{}\"\n",
                path_with_fallback(value, inherited.as_deref())
            ));
            continue;
        }
        // `Env` is refused in an authored tape — the environment belongs to
        // the scene — which is precisely why it is set here, from the
        // scene, in the one tape teleprompt writes itself.
        out.push_str(&format!("Env {key} \"{value}\"\n"));
    }
    out.push_str(&format!(
        "Sleep {}ms\n",
        session.number("settle_ms", 800).max(1)
    ));
    out.push_str("Show\n");

    for shot in &session.shots {
        out.push_str(shot.source.trim_end());
        out.push('\n');
    }
    out
}

/// A scene's `PATH`, with the one teleprompt was run with behind it.
///
/// A scene's `env` describes the terminal being recorded, and setting
/// `PATH` there is how a project points its tapes at the binary it is
/// demonstrating. But VHS applies `Env` to its *own* process, and finds
/// `ttyd` — and the browser it screenshots through — by searching `PATH`.
/// A scene that replaced `PATH` outright therefore disarmed the recorder,
/// and the failure arrived in the wrong vocabulary: `could not start tty:
/// exec: "ttyd": executable file not found in $PATH`, for a setting the
/// author wrote to make their own program findable.
///
/// So the scene wins where it speaks — its directories come first — and
/// what teleprompt inherited follows, so the tools VHS needs stay
/// reachable. A directory the scene already names is not repeated.
fn path_with_fallback(scene: &str, inherited: Option<&str>) -> String {
    let mut out: Vec<&str> = scene.split(':').filter(|d| !d.is_empty()).collect();
    for dir in inherited.unwrap_or_default().split(':') {
        if !dir.is_empty() && !out.contains(&dir) {
            out.push(dir);
        }
    }
    out.join(":")
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
    ) -> Result<Vec<Clip>, CaptureError> {
        std::fs::create_dir_all(out_dir).map_err(|source| CaptureError::Io {
            path: out_dir.display().to_string(),
            source,
        })?;
        let work = WorkDir::create(out_dir, "vhs").map_err(|source| CaptureError::Io {
            path: out_dir.display().to_string(),
            source,
        })?;

        let failed = |shot: &str, reason: String| CaptureError::Failed {
            backend: "vhs".to_string(),
            shot: shot.to_string(),
            reason,
        };
        let first = session
            .shots
            .first()
            .map(|s| s.id.clone())
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
        if let Some(why) = starved(&video, session) {
            return Err(failed(&first, why));
        }

        let wanted = session.wanted();
        let mut clips = Vec::new();
        for (shot, (from_ms, duration_ms)) in session.shots.iter().zip(windows(session)) {
            if !shot.wanted {
                continue;
            }
            let clip = out_dir.join(format!("{}.mp4", shot.key));
            cut(&self.ffmpeg, &video, &clip, from_ms, duration_ms)
                .map_err(|reason| failed(&shot.id, reason))?;
            clips.push(Clip {
                key: shot.key,
                path: clip,
            });
            on_progress(Progress {
                scene: session.scene.clone(),
                shot: shot.id.clone(),
                done: clips.len(),
                of: wanted,
            });
        }

        Ok(clips)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teleprompt_capture::SessionShot;
    use teleprompt_core::Hash;

    fn session(shots: &[(&str, u64)]) -> Session {
        Session {
            scene: "terminal".into(),
            adapter: "vhs".into(),
            name: None,
            settings: Default::default(),
            shots: shots
                .iter()
                .map(|(source, ms)| SessionShot {
                    id: "s".into(),
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

    /// The shots go in in order and unedited. They have already been
    /// re-timed to their slots; rewriting them here would be a second
    /// opinion about a number that is already settled.
    #[test]
    fn a_session_is_one_tape_of_its_shots_in_order() {
        let tape = tape_for(
            &session(&[("Type \"one\"\n", 500), ("Type \"two\"\n", 500)]),
            &frame(),
            "/out/session.mp4",
        );
        let one = tape.find("Type \"one\"").expect("the first shot is in it");
        let two = tape.find("Type \"two\"").expect("and the second");
        assert!(one < two, "in order:\n{tape}");
        assert!(
            tape.starts_with("Output \"/out/session.mp4\"\n"),
            "an unquoted path is three syntax errors to VHS's parser:\n{tape}"
        );
    }

    /// The shell's first prompt belongs to no shot. Recording it would put
    /// every shot late by however long it took to draw.
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
        let env = tape.find("Env PATH \"target/release").expect("set");
        assert!(env < tape.find("Show\n").unwrap(), "before the recording");
    }

    /// A scene's PATH says what its terminal should prefer, not what the
    /// recorder should forget.
    ///
    /// VHS applies `Env` to its own process, and finds both `ttyd` and the
    /// browser it screenshots through by searching PATH. A scene that
    /// replaced PATH outright therefore disarmed the recorder: the manual
    /// asks for `target/release:/usr/local/bin:/usr/bin:/bin`, and on a
    /// GitHub runner — where `vhs-action` installs `ttyd` somewhere else —
    /// that is a VHS which cannot open a terminal at all.
    ///
    /// So the scene's entries come first and win, and what teleprompt was
    /// run with follows as a fallback.
    #[test]
    fn a_scene_path_keeps_what_teleprompt_was_run_with_behind_it() {
        let composed = path_with_fallback("target/release", Some("/usr/bin:/opt/ttyd/bin"));
        assert_eq!(composed, "target/release:/usr/bin:/opt/ttyd/bin");

        // A directory the scene already names is not repeated.
        let composed = path_with_fallback("target/release:/usr/bin", Some("/usr/bin:/bin"));
        assert_eq!(composed, "target/release:/usr/bin:/bin");

        // Nothing inherited is nothing appended.
        assert_eq!(path_with_fallback("only", None), "only");
    }

    /// Only PATH is extended. A scene that sets any other variable means
    /// exactly what it says.
    #[test]
    fn other_scene_variables_are_passed_through_untouched() {
        let mut s = session(&[("Type \"x\"\n", 100)]);
        s.settings.insert("env.EDITOR".into(), "vim".into());
        let tape = tape_for(&s, &frame(), "o.mp4");
        assert!(tape.contains("Env EDITOR \"vim\"\n"), "{tape}");
    }

    /// The shots are windows onto the one video, at the offsets the tape
    /// was written to produce.
    #[test]
    fn the_shots_are_the_scheduled_durations_accumulated() {
        let windows = windows(&session(&[("a", 800), ("b", 1_200), ("c", 400)]));
        assert_eq!(windows, vec![(0, 800), (800, 1_200), (2_000, 400)]);
    }
}
