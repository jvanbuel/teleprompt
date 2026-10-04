//! Running a session on the Mac's own screen.
//!
//! macOS has no virtual display to hide a session on, so the app runs on
//! the author's screen, brought to the front, and the recording is that
//! screen cropped to the app's window. Don't use the Mac during a capture:
//! what it types goes to whatever is frontmost.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use teleprompt_desktop::run::{cut_clips, failed, get_ready, play, uses_pointer, Failure, Reel};
use teleprompt_plugin::capture::{
    CaptureBackend, CaptureError, Clip, Frame, Progress, Session, WorkDir,
};

use crate::jxa::{self, Process, Window};
use crate::PLUGIN_NAME;

const LAUNCH_TIMEOUT_MS: u32 = 30_000;
const LEAD_IN: Duration = Duration::from_millis(800);

/// Where the app's window is on the screen, in points, and the screen's
/// pixels per point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
}

/// Records `macos` scenes on the Mac's own screen.
#[derive(Debug, Clone)]
pub struct MacosRender {
    pub osascript: String,
    pub ffmpeg: String,
}

impl Default for MacosRender {
    fn default() -> Self {
        Self {
            osascript: "osascript".into(),
            ffmpeg: "ffmpeg".into(),
        }
    }
}

struct Owned(Child);

impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl CaptureBackend for MacosRender {
    fn unavailable(&self) -> Option<String> {
        if !cfg!(target_os = "macos") {
            return Some("it records a Mac's own screen, and this is not a Mac".into());
        }
        teleprompt_plugin::tool::missing(&[&self.ffmpeg])
    }

    fn needs(&self) -> &'static [&'static teleprompt_plugin::tool::Tool] {
        static NEEDS: &[&teleprompt_plugin::tool::Tool] = &[&teleprompt_plugin::tool::FFMPEG];
        NEEDS
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
        let fail = |why: String| failed(PLUGIN_NAME, (first.clone(), why));
        let io = |path: &Path| {
            let path = path.display().to_string();
            move |source| CaptureError::Io { path, source }
        };
        std::fs::create_dir_all(out_dir).map_err(io(out_dir))?;
        let work = WorkDir::create(out_dir, "macos").map_err(io(out_dir))?;
        let command = session.settings.get("command").ok_or_else(|| {
            fail(format!(
                "scene `{}` names no app: set `command` under [scene.{}], \
                 e.g. command = \"open -W -n -a Calculator\"",
                session.scene, session.scene
            ))
        })?;

        let mut app = launch(command, session, &work.join("app.log")).map_err(&fail)?;
        let process = match session.settings.get("process") {
            Some(name) => Process::Named(name.clone()),
            None => Process::Pid(app.0.id()),
        };
        let placed = self
            .place(&process, session, frame, &mut app)
            .map_err(&fail)?;
        let mut window = Window {
            osascript: self.osascript.clone(),
            process: process.clone(),
            origin: (placed.x, placed.y),
        };
        let timeout = Duration::from_millis(
            session
                .number("launch_timeout_ms", LAUNCH_TIMEOUT_MS)
                .into(),
        );
        get_ready(&mut window, session, timeout).map_err(&fail)?;
        let screen = screen_device(&self.ffmpeg, session.setting("screen", "Capture screen 0"))
            .map_err(&fail)?;

        let reel_path = work.join("reel.mkv");
        let input = [
            "-f",
            "avfoundation",
            "-capture_cursor",
            if uses_pointer(session) { "1" } else { "0" },
            "-capture_mouse_clicks",
            if uses_pointer(session) { "1" } else { "0" },
            "-framerate",
            &frame.fps.to_string(),
            "-i",
            &format!("{screen}:none"),
        ]
        .map(str::to_string);
        let reel = Reel::start(
            &self.ffmpeg,
            &input,
            Some(&crop(&placed, frame)),
            frame.fps,
            &reel_path,
        )
        .map_err(&fail)?;
        std::thread::sleep(LEAD_IN);
        let began = play(session, &mut window).map_err(|f: Failure| failed(PLUGIN_NAME, f))?;
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
        .map_err(|f| failed(PLUGIN_NAME, f))
    }
}

impl MacosRender {
    /// Waits for the app's window, fits it to the frame and lets it settle.
    fn place(
        &self,
        process: &Process,
        session: &Session,
        frame: &Frame,
        app: &mut Owned,
    ) -> Result<Placed, String> {
        let timeout = Duration::from_millis(
            session
                .number("launch_timeout_ms", LAUNCH_TIMEOUT_MS)
                .into(),
        );
        let start = Instant::now();
        loop {
            let count = jxa::run(&self.osascript, &jxa::count_windows(process))?;
            if count.parse::<u32>().unwrap_or(0) > 0 {
                break;
            }
            // `open -W` and a direct binary both keep running while the app
            // does; one that exits has failed.
            if let Ok(Some(status)) = app.0.try_wait() {
                return Err(format!("the app exited {status} before showing a window"));
            }
            if start.elapsed() > timeout {
                return Err(format!(
                    "the app showed no window in {}s",
                    timeout.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let placed = parse_placed(&jxa::run(
            &self.osascript,
            &jxa::fit(process, frame.width, frame.height),
        )?)?;
        Ok(placed)
    }
}

/// `{"x":0,"y":25,"w":1280,"h":720,"scale":2}`, as `jxa::fit` prints it.
pub fn parse_placed(json: &str) -> Result<Placed, String> {
    let number = |key: &str| -> Option<f64> {
        let at = json.find(&format!("\"{key}\":"))? + key.len() + 3;
        let rest = &json[at..];
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        rest[..end].trim().parse().ok()
    };
    match (
        number("x"),
        number("y"),
        number("w"),
        number("h"),
        number("scale"),
    ) {
        (Some(x), Some(y), Some(w), Some(h), Some(scale)) => Ok(Placed { x, y, w, h, scale }),
        _ => Err(format!("could not read the window's place from `{json}`")),
    }
}

/// The screen recording cropped to the window, in pixels, and fitted to
/// the frame: scaled down whole, never cut, and centred on black.
pub fn crop(placed: &Placed, frame: &Frame) -> String {
    let px = |v: f64| (v * placed.scale).round() as i64;
    // Even sizes: the encoder's chroma is subsampled by two.
    let even = |v: i64| v - v % 2;
    format!(
        "crop={}:{}:{}:{},scale={w}:{h}:force_original_aspect_ratio=decrease,\
         pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1",
        even(px(placed.w)),
        even(px(placed.h)),
        px(placed.x),
        px(placed.y),
        w = frame.width,
        h = frame.height,
    )
}

/// The avfoundation index of the screen named `name` ("Capture screen 0"),
/// from ffmpeg's own list of devices.
pub fn screen_device(ffmpeg: &str, name: &str) -> Result<String, String> {
    let out = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-f",
            "avfoundation",
            "-list_devices",
            "true",
            "-i",
            "",
        ])
        .output()
        .map_err(|e| format!("{ffmpeg} could not be run: {e}"))?;
    device_index(&String::from_utf8_lossy(&out.stderr), name).ok_or_else(|| {
        format!(
            "ffmpeg lists no screen called \"{name}\"; set `screen` to one of its \
             `Capture screen` devices (`ffmpeg -f avfoundation -list_devices true -i \"\"`)"
        )
    })
}

/// `[AVFoundation indev @ 0x…] [3] Capture screen 0` gives `3`.
pub fn device_index(listing: &str, name: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let (_, rest) = line.rsplit_once("] [")?;
        let (index, device) = rest.split_once("] ")?;
        (device.trim() == name).then(|| index.to_string())
    })
}

/// The scene's `command`, run by `sh` in the build's directory.
fn launch(command: &str, session: &Session, log: &Path) -> Result<Owned, String> {
    let output = std::fs::File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
    let errors = output
        .try_clone()
        .map_err(|e| format!("{}: {e}", log.display()))?;
    let mut run = Command::new("sh");
    run.arg("-c").arg(format!("exec {command}"));
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

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = "\
[AVFoundation indev @ 0x7f8] AVFoundation video devices:
[AVFoundation indev @ 0x7f8] [0] FaceTime HD Camera
[AVFoundation indev @ 0x7f8] [1] Capture screen 0
[AVFoundation indev @ 0x7f8] [2] Capture screen 1
[AVFoundation indev @ 0x7f8] AVFoundation audio devices:
[AVFoundation indev @ 0x7f8] [0] MacBook Pro Microphone
";

    #[test]
    fn a_screen_is_found_by_name_in_ffmpegs_list() {
        assert_eq!(
            device_index(LISTING, "Capture screen 0").as_deref(),
            Some("1")
        );
        assert_eq!(
            device_index(LISTING, "Capture screen 1").as_deref(),
            Some("2")
        );
        assert_eq!(device_index(LISTING, "Capture screen 2"), None);
    }

    #[test]
    fn the_window_is_cropped_in_pixels_and_fitted_whole() {
        let placed = parse_placed(r#"{"x":0,"y":25,"w":1280,"h":719,"scale":2}"#).unwrap();
        let frame = Frame {
            width: 1280,
            height: 720,
            fps: 30,
        };
        let filter = crop(&placed, &frame);
        assert!(
            filter.starts_with("crop=2560:1438:0:50,scale=1280:720:"),
            "{filter}"
        );
        assert!(
            filter.contains("force_original_aspect_ratio=decrease"),
            "never cut"
        );
    }

    #[test]
    fn a_place_that_cannot_be_read_says_what_it_got() {
        let why = parse_placed("execution error").unwrap_err();
        assert!(why.contains("execution error"), "{why}");
    }
}
