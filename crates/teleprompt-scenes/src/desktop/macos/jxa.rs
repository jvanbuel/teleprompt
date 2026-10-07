//! The window, driven with JavaScript for Automation.
//!
//! `osascript -l JavaScript` reaches both System Events (keys, windows)
//! and CoreGraphics (the pointer), so nothing beyond what macOS ships is
//! needed. Every script is built by a function here and tested as text;
//! running one needs a Mac, and the Accessibility permission for whatever
//! runs `teleprompt`.

use std::process::Command;
use std::time::Duration;

use crate::desktop::run::Screen;
use crate::desktop::script::{Button, Chord, Key, Modifier};

/// Runs a script, returning what it printed.
pub fn run(osascript: &str, script: &str) -> Result<String, String> {
    let out = Command::new(osascript)
        .args(["-l", "JavaScript", "-e", script])
        .output()
        .map_err(|e| format!("{osascript} could not be run: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let said = String::from_utf8_lossy(&out.stderr);
        let hint =
            if said.contains("not allowed") || said.contains("-1743") || said.contains("-25211") {
                " (give the terminal running teleprompt Accessibility access in System Settings \
             → Privacy & Security)"
            } else {
                ""
            };
        Err(format!(
            "osascript exited {}: {}{hint}",
            out.status,
            said.trim()
        ))
    }
}

/// A JavaScript string literal.
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The process whose windows a scene follows: the app's own, by the pid it
/// was started with, or by name when `process` is set (an app started
/// through `open` is not the pid `open` had).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Process {
    Pid(u32),
    Named(String),
}

impl Process {
    fn js(&self) -> String {
        match self {
            Process::Pid(pid) => {
                format!("Application(\"System Events\").processes.whose({{unixId: {pid}}})[0]")
            }
            Process::Named(name) => {
                format!(
                    "Application(\"System Events\").processes.byName({})",
                    quote(name)
                )
            }
        }
    }
}

/// How many windows the process has; `0` until it has started.
pub fn count_windows(process: &Process) -> String {
    format!(
        "(() => {{ try {{ return {}.windows.length; }} catch (e) {{ return 0; }} }})()",
        process.js()
    )
}

/// Brings the app forward and puts its first window at `(x, y)`, `w` by
/// `h` points, then prints where it really is: an app may refuse a size.
/// The scale is the screen's pixels per point, for cropping the recording.
pub fn fit(process: &Process, w: u32, h: u32) -> String {
    format!(
        r#"ObjC.import("AppKit");
const p = {process};
p.frontmost = true;
const screen = $.NSScreen.mainScreen;
const full = screen.frame, seen = screen.visibleFrame;
const top = full.size.height - (seen.origin.y + seen.size.height);
const win = p.windows[0];
win.position = [0, top];
win.size = [{w}, {h}];
delay(0.3);
const at = win.position(), size = win.size();
JSON.stringify({{x: at[0], y: at[1], w: size[0], h: size[1], scale: screen.backingScaleFactor}});"#,
        process = process.js(),
    )
}

/// The window's title.
pub fn title(process: &Process) -> String {
    format!("{}.windows[0].name()", process.js())
}

/// Presses a chord in whatever is frontmost.
pub fn press(chord: &Chord) -> String {
    let using: Vec<String> = chord
        .modifiers
        .iter()
        .map(|m| {
            quote(match m {
                Modifier::Ctrl => "control down",
                Modifier::Alt => "option down",
                Modifier::Shift => "shift down",
                Modifier::Super => "command down",
            })
        })
        .collect();
    let using = format!("{{using: [{}]}}", using.join(", "));
    match &chord.key {
        Key::Named(name) => format!(
            "Application(\"System Events\").keyCode({}, {using});",
            key_code(name)
        ),
        Key::Char(c) => format!(
            "Application(\"System Events\").keystroke({}, {using});",
            quote(&c.to_string())
        ),
    }
}

/// macOS's virtual key code for a named key.
pub fn key_code(name: &str) -> u32 {
    match name {
        "Enter" => 36,
        "Tab" => 48,
        "Space" => 49,
        "Backspace" => 51,
        "Escape" => 53,
        "Delete" => 117,
        "Left" => 123,
        "Right" => 124,
        "Down" => 125,
        "Up" => 126,
        "Home" => 115,
        "End" => 119,
        "PageUp" => 116,
        "PageDown" => 121,
        "F1" => 122,
        "F2" => 120,
        "F3" => 99,
        "F4" => 118,
        "F5" => 96,
        "F6" => 97,
        "F7" => 98,
        "F8" => 100,
        "F9" => 101,
        "F10" => 109,
        "F11" => 103,
        "F12" => 111,
        other => unreachable!("`{other}` is not in crate::desktop::script::KEYS"),
    }
}

/// Types `text` a character at a time, `gap` apart.
pub fn type_text(text: &str, gap: Duration) -> String {
    format!(
        "const se = Application(\"System Events\");\n\
         for (const c of {}) {{ se.keystroke(c); delay({:.3}); }}",
        quote(text),
        gap.as_secs_f64()
    )
}

/// The pointer, glided from wherever it is to `to` over `over`, through
/// CoreGraphics: System Events cannot move it. Event types are
/// CoreGraphics' numbers (`kCGEventMouseMoved` is 5), not names the bridge
/// may not carry.
pub fn glide(to: (f64, f64), over: Duration) -> String {
    let steps = (over.as_millis() / 20).clamp(1, 60);
    format!(
        r#"ObjC.import("CoreGraphics");
const here = $.CGEventGetLocation($.CGEventCreate(null));
const from = [here.x, here.y], to = [{}, {}], n = {steps};
for (let i = 1; i <= n; i++) {{
  const t = i / n, e = t * t * (3 - 2 * t);
  const at = {{x: from[0] + (to[0] - from[0]) * e, y: from[1] + (to[1] - from[1]) * e}};
  $.CGEventPost(0, $.CGEventCreateMouseEvent(null, 5, at, 0));
  delay({:.3});
}}"#,
        to.0,
        to.1,
        over.as_secs_f64() / steps as f64
    )
}

/// A press and release where the pointer is; a double click is two, the
/// second marked as such (`kCGMouseEventClickState`, field 1).
pub fn click(button: Button, double: bool) -> String {
    let (down, up, which) = match button {
        Button::Left => (1, 2, 0),
        Button::Right => (3, 4, 1),
    };
    let times = if double { 2 } else { 1 };
    format!(
        r#"ObjC.import("CoreGraphics");
const at = $.CGEventGetLocation($.CGEventCreate(null));
for (let n = 1; n <= {times}; n++) {{
  for (const type of [{down}, {up}]) {{
    const e = $.CGEventCreateMouseEvent(null, type, at, {which});
    $.CGEventSetIntegerValueField(e, 1, n);
    $.CGEventPost(0, e);
  }}
}}"#
    )
}

/// The app's window, with where it is on the screen, in points.
pub struct Window {
    pub osascript: String,
    pub process: Process,
    pub origin: (f64, f64),
}

impl Screen for Window {
    fn press(&mut self, chord: &Chord) -> Result<(), String> {
        run(&self.osascript, &press(chord)).map(drop)
    }

    fn type_text(&mut self, text: &str, gap: Duration) -> Result<(), String> {
        run(&self.osascript, &type_text(text, gap)).map(drop)
    }

    fn point(&mut self, x: u32, y: u32, over: Duration) -> Result<(), String> {
        let to = (self.origin.0 + f64::from(x), self.origin.1 + f64::from(y));
        run(&self.osascript, &glide(to, over)).map(drop)
    }

    fn click(&mut self, button: Button, double: bool) -> Result<(), String> {
        run(&self.osascript, &click(button, double)).map(drop)
    }

    fn title(&mut self) -> Result<String, String> {
        run(&self.osascript, &title(&self.process))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::script::{classify, Action, KEYS};

    fn chord(line: &str) -> Chord {
        match classify(line) {
            Ok(Action::Press { chord, .. }) => chord,
            other => panic!("{line}: {other:?}"),
        }
    }

    #[test]
    fn every_named_key_has_a_key_code() {
        for key in KEYS {
            key_code(key);
        }
    }

    #[test]
    fn chords_press_through_system_events() {
        assert_eq!(
            press(&chord("Cmd+Shift+Space")),
            "Application(\"System Events\").keyCode(49, {using: [\"command down\", \"shift down\"]});"
        );
        assert_eq!(
            press(&chord("Key e")),
            "Application(\"System Events\").keystroke(\"e\", {using: []});"
        );
        assert!(press(&chord("Ctrl+\"")).contains(r#"keystroke("\"""#));
    }

    #[test]
    fn typed_text_is_a_safe_string_literal() {
        let script = type_text("say \"hi\"\\", Duration::from_millis(40));
        assert!(
            script.contains(r#"for (const c of "say \"hi\"\\")"#),
            "{script}"
        );
        assert!(script.contains("delay(0.040)"), "{script}");
    }

    #[test]
    fn a_process_is_found_by_pid_or_by_name() {
        assert!(title(&Process::Pid(42)).contains("whose({unixId: 42})[0]"));
        assert!(title(&Process::Named("Tele\"prompt".into())).contains(r#"byName("Tele\"prompt")"#));
    }

    #[test]
    fn a_glide_ends_on_its_point_and_a_double_click_counts_to_two() {
        let g = glide((100.0, 50.0), Duration::from_millis(400));
        assert!(g.contains("to = [100, 50], n = 20"), "{g}");
        let c = click(Button::Right, true);
        assert!(c.contains("n <= 2") && c.contains("[3, 4]"), "{c}");
    }
}
