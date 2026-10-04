//! Running a session: a display, the app on it, the shots, the reel.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use teleprompt_desktop::run::{cut_clips, failed, get_ready, play, uses_pointer, Failure, Reel};
use teleprompt_plugin::capture::{
    CaptureBackend, CaptureError, Clip, Frame, Progress, Session, WorkDir,
};

use crate::xdo::{xdotool, Window};
use crate::ADAPTER;

/// How long the app has to show a window, unless `launch_timeout_ms` says.
const LAUNCH_TIMEOUT_MS: u32 = 30_000;

/// Recording before the first shot, so a shot never begins before its
/// reel does.
const LEAD_IN: Duration = Duration::from_millis(600);

/// Records `x11` scenes on a virtual display.
#[derive(Debug, Clone)]
pub struct X11Render {
    pub xvfb: String,
    pub xdotool: String,
    pub ffmpeg: String,
}

impl Default for X11Render {
    fn default() -> Self {
        Self {
            xvfb: "Xvfb".into(),
            xdotool: "xdotool".into(),
            ffmpeg: "ffmpeg".into(),
        }
    }
}

/// A child process killed when dropped: the display, the app.
struct Owned(Child);

impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl CaptureBackend for X11Render {
    fn unavailable(&self) -> Option<String> {
        teleprompt_plugin::tool::missing(&[&self.xvfb, &self.xdotool, &self.ffmpeg])
    }

    fn needs(&self) -> &'static [&'static str] {
        &["Xvfb", "xdotool", "ffmpeg"]
    }

    fn capture(
        &self,
        session: &Session,
        frame: &Frame,
        out_dir: &Path,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Clip>, CaptureError> {
        let first = session
            .shots
            .first()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| "?".into());
        let fail = |why: String| failed(ADAPTER, (first.clone(), why));
        let io = |path: &Path| {
            let path = path.display().to_string();
            move |source| CaptureError::Io { path, source }
        };
        std::fs::create_dir_all(out_dir).map_err(io(out_dir))?;
        let work = WorkDir::create(out_dir, "x11").map_err(io(out_dir))?;

        let command = session.settings.get("command").ok_or_else(|| {
            fail(format!(
                "scene `{}` names no app: set `command` under [scene.{}], \
                 e.g. command = \"gnome-calculator\"",
                session.scene, session.scene
            ))
        })?;

        let (_display, display) = self.display(frame).map_err(&fail)?;
        let log = work.join("app.log");
        let mut app = launch(command, &display, session, &log).map_err(&fail)?;
        let mut window = self
            .window(&display, session, frame, &mut app)
            .map_err(|why| fail(with_log(why, &log)))?;
        get_ready(&mut window, session, launch_timeout(session))
            .map_err(|why| fail(with_log(why, &log)))?;

        let reel_path = work.join("reel.mkv");
        let input = [
            "-f",
            "x11grab",
            "-draw_mouse",
            if uses_pointer(session) { "1" } else { "0" },
            "-framerate",
            &frame.fps.to_string(),
            "-video_size",
            &format!("{}x{}", frame.width, frame.height),
            "-i",
            &display,
        ]
        .map(str::to_string);
        let reel = Reel::start(&self.ffmpeg, &input, None, frame.fps, &reel_path).map_err(&fail)?;
        std::thread::sleep(LEAD_IN);
        let began = play(session, &mut window).map_err(|f: Failure| failed(ADAPTER, f))?;
        std::thread::sleep(Duration::from_millis(200));
        reel.stop().map_err(&fail)?;

        cut_clips(
            &self.ffmpeg,
            &reel_path,
            session,
            &began,
            out_dir,
            on_progress,
        )
        .map_err(|f| failed(ADAPTER, f))
    }
}

impl X11Render {
    /// A display of its own at the frame's size: `-displayfd` picks a free
    /// number and says which.
    fn display(&self, frame: &Frame) -> Result<(Owned, String), String> {
        let mut child = Command::new(&self.xvfb)
            .args(["-displayfd", "1", "-nolisten", "tcp", "-br", "-screen", "0"])
            .arg(format!("{}x{}x24", frame.width, frame.height))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{} could not be run: {e}", self.xvfb))?;
        let stdout = child.stdout.take().expect("piped");
        let owned = Owned(child);
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .map_err(|e| format!("{}: {e}", self.xvfb))?;
        let number = line.trim();
        if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("{} started no display", self.xvfb));
        }
        Ok((owned, format!(":{number}")))
    }

    /// The app's window, once it shows: fitted to the display, or centred
    /// on it if the app will not be that size, then left to settle.
    fn window(
        &self,
        display: &str,
        session: &Session,
        frame: &Frame,
        app: &mut Owned,
    ) -> Result<Window, String> {
        let title = session.setting("title", "");
        let timeout = launch_timeout(session);
        let start = Instant::now();
        let id = loop {
            if let Some(id) = self.largest(display, title) {
                break id;
            }
            if let Ok(Some(status)) = app.0.try_wait() {
                return Err(format!("the app exited {status} before showing a window"));
            }
            if start.elapsed() > timeout {
                return Err(match title {
                    "" => format!("the app showed no window in {}s", timeout.as_secs()),
                    t => format!(
                        "no window titled \"{t}\" appeared in {}s",
                        timeout.as_secs()
                    ),
                });
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        let size = (frame.width.to_string(), frame.height.to_string());
        let run = |args: &[&str]| xdotool(&self.xdotool, display, args);
        run(&["windowsize", &id, &size.0, &size.1])?;
        run(&["windowmove", &id, "0", "0"])?;
        std::thread::sleep(Duration::from_millis(300));
        let (w, h) = self.size(display, &id)?;
        let origin = (
            (i32::try_from(frame.width).unwrap_or(0) - w).max(0) / 2,
            (i32::try_from(frame.height).unwrap_or(0) - h).max(0) / 2,
        );
        run(&[
            "windowmove",
            &id,
            &origin.0.to_string(),
            &origin.1.to_string(),
        ])?;
        run(&["windowactivate", "--sync", &id]).or_else(|_| run(&["windowfocus", &id]))?;
        // Where a block's first glide starts; drawn only in a session that
        // moves it (`uses_pointer`).
        let pointer = (origin.0 + w / 2, origin.1 + h / 2);
        run(&["mousemove", &pointer.0.to_string(), &pointer.1.to_string()])?;
        Ok(Window {
            xdotool: self.xdotool.clone(),
            display: display.to_string(),
            id,
            origin,
            pointer,
        })
    }

    /// The biggest visible window whose title contains `title`: an app's
    /// main window, not its tooltips.
    fn largest(&self, display: &str, title: &str) -> Option<String> {
        let ids = xdotool(
            &self.xdotool,
            display,
            &["search", "--onlyvisible", "--name", title],
        )
        .ok()?;
        // The display's own root window matches any search.
        let root = xdotool(
            &self.xdotool,
            display,
            &["search", "--maxdepth", "0", "--name", ""],
        )
        .ok();
        ids.lines()
            .filter(|id| root.as_deref() != Some(*id))
            .filter_map(|id| Some((self.size(display, id).ok()?, id.to_string())))
            .filter(|((w, h), _)| *w > 50 && *h > 50)
            .max_by_key(|((w, h), _)| w * h)
            .map(|(_, id)| id)
    }

    fn size(&self, display: &str, id: &str) -> Result<(i32, i32), String> {
        let shell = xdotool(
            &self.xdotool,
            display,
            &["getwindowgeometry", "--shell", id],
        )?;
        let value = |key: &str| {
            shell
                .lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
        };
        value("WIDTH")
            .zip(value("HEIGHT"))
            .ok_or_else(|| format!("no size for window {id}: {shell}"))
    }
}

/// The scene's `command`, run by `sh` in the build's directory on
/// `display`, inside its own D-Bus session where `dbus-run-session` is
/// installed: a GTK or Qt app expects one, and should not find the
/// author's.
fn launch(command: &str, display: &str, session: &Session, log: &Path) -> Result<Owned, String> {
    let output = std::fs::File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
    let errors = output
        .try_clone()
        .map_err(|e| format!("{}: {e}", log.display()))?;
    let dbus = session.setting("dbus", "true") != "false"
        && teleprompt_plugin::tool::installed("dbus-run-session");
    let mut run = if dbus {
        let mut c = Command::new("dbus-run-session");
        c.args(["--", "sh", "-c"]);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c");
        c
    };
    run.arg(format!("exec {command}"))
        .env("DISPLAY", display)
        .env("GDK_BACKEND", "x11")
        .env("QT_QPA_PLATFORM", "xcb")
        .env_remove("WAYLAND_DISPLAY");
    for (key, value) in session.nested("env") {
        run.env(key, value);
    }
    run.stdin(Stdio::null())
        .stdout(output)
        .stderr(errors)
        .spawn()
        .map(Owned)
        .map_err(|e| format!("`{command}` could not be started: {e}"))
}

/// `why`, and what the app said last, which is usually the reason.
fn with_log(why: String, log: &Path) -> String {
    let said = std::fs::read_to_string(log).unwrap_or_default();
    match teleprompt_plugin::tool::tail(&said, 4) {
        tail if tail.is_empty() => why,
        tail => format!("{why}; it said: {tail}"),
    }
}

fn launch_timeout(session: &Session) -> Duration {
    Duration::from_millis(
        session
            .number("launch_timeout_ms", LAUNCH_TIMEOUT_MS)
            .into(),
    )
}
