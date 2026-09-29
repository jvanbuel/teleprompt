//! Recording a terminal session by handing `vhs` one re-timed tape.
//!
//! Every shot is already re-timed to its slot, so the tape *is* the
//! schedule. A session's shots are concatenated so the program keeps
//! running across them, and each shot is cut from the one video at its
//! scheduled offset (see `docs/design.md#capture`).

use std::path::Path;
use std::process::{Command, Stdio};

use teleprompt_capture::reel::{cut, starved, windows};
use teleprompt_capture::{CaptureBackend, CaptureError, Clip, Frame, Progress, Session, WorkDir};

/// The tape `vhs` runs for a whole session, writing its video to `output`.
pub(crate) fn tape_for(session: &Session, frame: &Frame, output: &str) -> String {
    let mut out = String::new();
    // Quoted: VHS's parser reads a bare `/` as the start of a command, so
    // an unquoted absolute path is three syntax errors and no recording.
    out.push_str(&format!("Output \"{output}\"\n"));

    // VHS sizes its window in pixels, the unit the render's frame is in.
    out.push_str(&format!("Set Width {}\n", frame.width));
    out.push_str(&format!("Set Height {}\n", frame.height));
    out.push_str(&format!("Set Framerate {}\n", frame.fps));
    if let Some(theme) = session.settings.get("theme") {
        out.push_str(&format!("Set Theme \"{theme}\"\n"));
    }
    if let Some(size) = session.settings.get("font_size") {
        out.push_str(&format!("Set FontSize {size}\n"));
    }

    // The shell drawing its first prompt belongs to no shot; recording it
    // would put every shot late by however long it took.
    out.push_str("Hide\n");
    for (key, value) in session.nested("env") {
        // See `path_with_fallback`.
        if key == "PATH" {
            let inherited = std::env::var("PATH").ok();
            out.push_str(&format!(
                "Env PATH \"{}\"\n",
                path_with_fallback(value, inherited.as_deref())
            ));
            continue;
        }
        // `Env` is refused in an authored tape because the environment is
        // the scene's; this is where the scene's is set.
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
/// VHS applies `Env` to its *own* process and finds `ttyd` and its browser
/// through `PATH`, so a scene that replaced `PATH` outright would leave VHS
/// unable to open a terminal. The scene's directories come first and win;
/// inherited ones follow, without repeats.
fn path_with_fallback(scene: &str, inherited: Option<&str>) -> String {
    let mut out: Vec<&str> = scene.split(':').filter(|d| !d.is_empty()).collect();
    for dir in inherited.unwrap_or_default().split(':') {
        if !dir.is_empty() && !out.contains(&dir) {
            out.push(dir);
        }
    }
    out.join(":")
}

/// Records `vhs` scenes by handing a re-timed tape to `vhs`.
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
    fn adapter(&self) -> &'static str {
        "vhs"
    }

    fn unavailable(&self) -> Option<String> {
        // Only a PATH check: this runs on every capture and every `doctor`,
        // and a probe recording would take a minute. A `vhs` that is
        // installed but records nothing is caught in `capture`, and its
        // session renders as slates with the reason.
        teleprompt_capture::tool::missing(&[&self.vhs, "ttyd", &self.ffmpeg])
    }

    fn needs(&self) -> &'static [&'static str] {
        &["vhs", "ttyd", "ffmpeg"]
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
            shot: shot.into(),
            reason,
        };
        let first = session
            .shots
            .first()
            .map(|s| s.id.to_string())
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

        // Output is captured, not inherited, so its tail can go into the
        // error: it is the only clue to a failed recording.
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
        // `vhs` can exit 0 and write no file.
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

        // The windows predict the tape's length; check it against the
        // recording before cutting.
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

    /// The shots go in in order and unedited: they are already re-timed.
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

    /// The shell's first prompt belongs to no shot.
    #[test]
    fn the_shell_starting_up_is_hidden() {
        let tape = tape_for(&session(&[("Type \"x\"\n", 100)]), &frame(), "o.mp4");
        let hide = tape.find("Hide\n").expect("a hidden prelude");
        let show = tape.find("Show\n").expect("that ends");
        let first = tape.find("Type \"x\"").unwrap();
        assert!(hide < show && show < first, "{tape}");
    }

    /// The scene's environment is set before the recording starts.
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
    /// recorder should forget: see [`path_with_fallback`].
    #[test]
    fn a_scene_path_keeps_what_teleprompt_was_run_with_behind_it() {
        let composed = path_with_fallback("target/release", Some("/usr/bin:/opt/ttyd/bin"));
        assert_eq!(composed, "target/release:/usr/bin:/opt/ttyd/bin");

        let composed = path_with_fallback("target/release:/usr/bin", Some("/usr/bin:/bin"));
        assert_eq!(composed, "target/release:/usr/bin:/bin");

        assert_eq!(path_with_fallback("only", None), "only");
    }

    /// Only PATH is extended.
    #[test]
    fn other_scene_variables_are_passed_through_untouched() {
        let mut s = session(&[("Type \"x\"\n", 100)]);
        s.settings.insert("env.EDITOR".into(), "vim".into());
        let tape = tape_for(&s, &frame(), "o.mp4");
        assert!(tape.contains("Env EDITOR \"vim\"\n"), "{tape}");
    }

    /// The shots are windows onto the one video at their scheduled offsets.
    #[test]
    fn the_shots_are_the_scheduled_durations_accumulated() {
        let windows = windows(&session(&[("a", 800), ("b", 1_200), ("c", 400)]));
        assert_eq!(windows, vec![(0, 800), (800, 1_200), (2_000, 400)]);
    }
}
