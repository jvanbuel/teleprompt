//! Playing a session's shots against a window, recording, and cutting.
//!
//! A desktop app's timing is its own: it launches, redraws and answers in
//! its own time. So clips are not cut at predicted offsets, as a re-timed
//! tape's are, but where each shot really began. The reel is recorded with
//! wall-clock timestamps, the runner notes the wall clock as each shot
//! starts, and the difference is the cut.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use teleprompt_core::ShotId;
use teleprompt_plugin::capture::{reel, CaptureError, Clip, Progress, Session};

use crate::script::{classify, Action, Button, Chord, Pace};

/// What a platform's plugin does to its app's window. Points are in the
/// window, from its top-left corner.
pub trait Screen {
    fn press(&mut self, chord: &Chord) -> Result<(), String>;
    /// Types `text`, a character every `gap`, and returns when done.
    fn type_text(&mut self, text: &str, gap: Duration) -> Result<(), String>;
    /// Glides the pointer to `(x, y)` over `over`.
    fn point(&mut self, x: u32, y: u32, over: Duration) -> Result<(), String>;
    fn click(&mut self, button: Button, double: bool) -> Result<(), String>;
    /// The window's title, for `Wait`.
    fn title(&mut self) -> Result<String, String>;
}

/// Whether any shot of the session moves the pointer. One that never does
/// is recorded without it: a still pointer over the app is clutter.
pub fn uses_pointer(session: &Session) -> bool {
    session.shots.iter().any(|shot| {
        shot.source
            .lines()
            .any(|l| matches!(classify(l), Ok(Action::Pointer { .. })))
    })
}

/// A shot that failed, and why.
pub type Failure = (ShotId, String);

/// Plays every shot of `session` in order, each padded to its scheduled
/// length, and returns when each began.
pub fn play(session: &Session, screen: &mut dyn Screen) -> Result<Vec<SystemTime>, Failure> {
    let mut began = Vec::with_capacity(session.shots.len());
    for shot in &session.shots {
        began.push(SystemTime::now());
        let start = Instant::now();
        perform(&shot.source, screen).map_err(|why| (shot.id.clone(), why))?;
        // Held, not frozen: the app stays live until the slot ends.
        let slot = Duration::from_millis(shot.duration_ms);
        if let Some(left) = slot.checked_sub(start.elapsed()) {
            std::thread::sleep(left);
        }
    }
    Ok(began)
}

/// One shot's actions. Each action has a cost, and the runner sleeps to
/// where the costs say it should be, so a slow `xdotool` or `osascript`
/// eats into its own pause rather than pushing every later action late.
fn perform(source: &str, screen: &mut dyn Screen) -> Result<(), String> {
    let origin = Instant::now();
    let mut due = Duration::ZERO;
    let mut pace = Pace::default();
    let catch_up = |due: Duration| {
        if let Some(left) = due.checked_sub(origin.elapsed()) {
            std::thread::sleep(left);
        }
    };
    for line in source.lines() {
        let Ok(action) = classify(line) else { continue };
        let before = pace;
        let cost = Duration::from_millis(pace.cost(&action));
        match &action {
            Action::Type { text, speed } => {
                let gap = Duration::from_millis(speed.unwrap_or(before.typing));
                screen.type_text(text, gap)?;
            }
            Action::Press {
                chord,
                count,
                speed,
            } => {
                let gap = Duration::from_millis(speed.unwrap_or(before.typing));
                for _ in 0..*count {
                    screen.press(chord)?;
                    due += gap;
                    catch_up(due);
                }
                continue;
            }
            Action::Pointer {
                x,
                y,
                button,
                double,
                ..
            } => {
                screen.point(*x, *y, cost)?;
                if let Some(button) = button {
                    screen.click(*button, *double)?;
                }
            }
            Action::Wait { text, .. } => {
                wait_for_title(screen, text, cost)?;
                // A wait costs what it took, not its bound.
                due = origin.elapsed();
                continue;
            }
            _ => {}
        }
        due += cost;
        catch_up(due);
    }
    Ok(())
}

/// Before recording: until the title contains the scene's `ready`, if it
/// sets one, so an app's loading is not part of the video; then `settle_ms`
/// for it to finish drawing.
pub fn get_ready(
    screen: &mut dyn Screen,
    session: &Session,
    timeout: Duration,
) -> Result<(), String> {
    if let Some(ready) = session.settings.get("ready") {
        wait_for_title(screen, ready, timeout)?;
    }
    std::thread::sleep(Duration::from_millis(
        session.number("settle_ms", SETTLE_MS).into(),
    ));
    Ok(())
}

/// How long a window is left to draw itself before recording, unless the
/// scene's `settle_ms` says.
pub const SETTLE_MS: u32 = 1_500;

fn wait_for_title(screen: &mut dyn Screen, text: &str, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        let title = screen.title()?;
        if title.contains(text) {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            return Err(format!(
                "the window's title never contained \"{text}\" in {:.1}s; it read \"{title}\"",
                timeout.as_secs_f64()
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// ffmpeg recording the screen into a reel whose timestamps are the wall
/// clock, until [`Reel::stop`].
pub struct Reel {
    child: Option<Child>,
}

impl Reel {
    /// Starts `ffmpeg` on `input` (its `-f … -i …` arguments), filtered by
    /// `filter` if given, writing `path` at `fps`.
    pub fn start(
        ffmpeg: &str,
        input: &[String],
        filter: Option<&str>,
        fps: u32,
        path: &Path,
    ) -> Result<Reel, String> {
        let mut command = Command::new(ffmpeg);
        command
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-use_wallclock_as_timestamps", "1"])
            .args(input)
            .args(["-copyts", "-r", &fps.to_string()]);
        if let Some(filter) = filter {
            command.args(["-vf", filter]);
        }
        let child = command
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "16"])
            .args(["-pix_fmt", "yuv420p"])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{ffmpeg} could not be run: {e}"))?;
        Ok(Reel { child: Some(child) })
    }

    /// Stops the way its own `q` key does, so the file is finished.
    pub fn stop(mut self) -> Result<(), String> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(b"q");
        }
        let out = child
            .wait_with_output()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        // `q` ends with 0; an earlier failure says why on stderr.
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "ffmpeg recording the screen exited {}: {}",
                out.status,
                teleprompt_plugin::tool::tail(&String::from_utf8_lossy(&out.stderr), 4)
            ))
        }
    }
}

impl Drop for Reel {
    fn drop(&mut self) {
        // Failing before `stop`: not left recording.
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The wall-clock time of a reel's first frame.
pub fn reel_start(reel: &Path) -> Option<SystemTime> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=start_time"])
        .args(["-of", "default=nw=1:nk=1"])
        .arg(reel)
        .output()
        .ok()?;
    let seconds: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(UNIX_EPOCH + Duration::from_secs_f64(seconds))
}

/// Cuts each wanted shot from the reel where it began.
pub fn cut_clips(
    ffmpeg: &str,
    reel: &Path,
    session: &Session,
    began: &[SystemTime],
    out_dir: &Path,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<Vec<Clip>, Failure> {
    let first = || {
        session
            .shots
            .first()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| ShotId::from("?"))
    };
    let start = reel_start(reel).ok_or_else(|| {
        (
            first(),
            format!("{} has no start time: nothing was recorded", reel.display()),
        )
    })?;
    let wanted = session.wanted();
    let mut clips = Vec::new();
    for (shot, at) in session.shots.iter().zip(began) {
        if !shot.wanted {
            continue;
        }
        // A shot can only begin before the reel if recording started late;
        // its first moments are then lost, not invented.
        let from_ms = at
            .duration_since(start)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        let clip = out_dir.join(format!("{}.mp4", shot.key));
        reel::cut(ffmpeg, reel, &clip, from_ms, shot.duration_ms)
            .map_err(|why| (shot.id.clone(), why))?;
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

/// A [`Failure`] as the capture error the build reports.
pub fn failed(backend: &str, (shot, reason): Failure) -> CaptureError {
    CaptureError::Failed {
        backend: backend.to_string(),
        shot,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teleprompt_core::Hash;
    use teleprompt_plugin::capture::SessionShot;

    /// A screen that writes down what it was asked to do, and when.
    #[derive(Default)]
    struct Log {
        start: Option<Instant>,
        did: Vec<(u128, String)>,
        title: String,
    }

    impl Log {
        fn note(&mut self, what: String) {
            let at = self
                .start
                .get_or_insert_with(Instant::now)
                .elapsed()
                .as_millis();
            self.did.push((at, what));
        }
    }

    impl Screen for Log {
        fn press(&mut self, chord: &Chord) -> Result<(), String> {
            self.note(format!("press {:?}", chord.key));
            Ok(())
        }
        fn type_text(&mut self, text: &str, gap: Duration) -> Result<(), String> {
            self.note(format!("type {text}"));
            std::thread::sleep(gap * u32::try_from(text.len()).unwrap());
            Ok(())
        }
        fn point(&mut self, x: u32, y: u32, over: Duration) -> Result<(), String> {
            self.note(format!("point {x},{y}"));
            std::thread::sleep(over);
            Ok(())
        }
        fn click(&mut self, _: Button, _: bool) -> Result<(), String> {
            self.note("click".into());
            Ok(())
        }
        fn title(&mut self) -> Result<String, String> {
            Ok(self.title.clone())
        }
    }

    fn session(shots: &[(&str, u64)]) -> Session {
        Session {
            scene: "app".into(),
            adapter: "x11".into(),
            name: None,
            settings: Default::default(),
            shots: shots
                .iter()
                .enumerate()
                .map(|(i, (source, ms))| SessionShot {
                    id: format!("s#{i}").as_str().into(),
                    key: Hash::of(source.as_bytes()),
                    source: (*source).to_string(),
                    duration_ms: *ms,
                    wanted: true,
                })
                .collect(),
        }
    }

    #[test]
    fn the_pointer_is_drawn_only_where_a_block_moves_it() {
        assert!(!uses_pointer(&session(&[
            ("Key a\n", 100),
            ("Sleep 1s\n", 100)
        ])));
        assert!(uses_pointer(&session(&[
            ("Key a\n", 100),
            ("Click 1 2\n", 100)
        ])));
    }

    #[test]
    fn actions_run_in_order_at_their_cost() {
        let mut log = Log::default();
        perform("Key a\nSleep 200ms\nClick@100ms 5 6\nEnter\n", &mut log).unwrap();
        let what: Vec<&str> = log.did.iter().map(|(_, w)| w.as_str()).collect();
        assert_eq!(
            what,
            [
                "press Char('a')",
                "point 5,6",
                "click",
                "press Named(\"Enter\")"
            ]
        );
        // 50ms for the key, then the sleep, before the pointer moves.
        assert!(log.did[1].0 >= 240, "{:?}", log.did);
    }

    /// Each shot fills its slot, so the next starts where the schedule
    /// put it.
    #[test]
    fn a_short_shot_is_held_to_its_slot() {
        let mut log = Log::default();
        let began = play(&session(&[("Key a\n", 300), ("Key b\n", 100)]), &mut log).unwrap();
        let gap = began[1].duration_since(began[0]).unwrap().as_millis();
        assert!((300..400).contains(&gap), "{gap}ms");
    }

    #[test]
    fn a_wait_returns_when_the_title_says_so_and_fails_when_it_never_does() {
        let mut log = Log {
            title: "tour.md - Teleprompt".into(),
            ..Log::default()
        };
        perform("Wait@1s \"tour.md\"\n", &mut log).unwrap();
        let why = perform("Wait@200ms \"other.md\"\n", &mut log).unwrap_err();
        assert!(why.contains("never contained \"other.md\""), "{why}");
    }
}
