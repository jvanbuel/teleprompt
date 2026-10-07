//! The window, driven with `xdotool`.

use std::process::Command;
use std::time::Duration;

use crate::desktop::run::Screen;
use crate::desktop::script::{Button, Chord, Key, Modifier};

/// The app's window on display `display`.
pub struct Window {
    pub xdotool: String,
    pub display: String,
    pub id: String,
    /// Where the window's top-left corner is on the display.
    pub origin: (i32, i32),
    /// Where the pointer is, on the display.
    pub pointer: (i32, i32),
}

impl Window {
    /// Runs `xdotool args…` on the display, returning what it printed.
    pub fn xdotool(&self, args: &[&str]) -> Result<String, String> {
        xdotool(&self.xdotool, &self.display, args)
    }
}

pub fn xdotool(program: &str, display: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .env("DISPLAY", display)
        .output()
        .map_err(|e| format!("{program} could not be run: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!(
            "`{program} {}` exited {}: {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

impl Screen for Window {
    fn press(&mut self, chord: &Chord) -> Result<(), String> {
        let keys = keysym(chord);
        self.xdotool(&["key", "--clearmodifiers", &keys]).map(drop)
    }

    fn type_text(&mut self, text: &str, gap: Duration) -> Result<(), String> {
        let delay = gap.as_millis().to_string();
        self.xdotool(&["type", "--delay", &delay, "--", text])
            .map(drop)
    }

    /// A glide of short steps, one `xdotool` run: the pointer is seen to
    /// travel, as a hand moves it.
    fn point(&mut self, x: u32, y: u32, over: Duration) -> Result<(), String> {
        let to = (
            self.origin.0 + i32::try_from(x).unwrap_or(i32::MAX),
            self.origin.1 + i32::try_from(y).unwrap_or(i32::MAX),
        );
        let steps = glide(self.pointer, to, over);
        let mut args: Vec<String> = Vec::new();
        for (i, (px, py)) in steps.iter().enumerate() {
            if i > 0 {
                args.push("sleep".into());
                args.push(format!("{:.3}", over.as_secs_f64() / steps.len() as f64));
            }
            args.extend(["mousemove".into(), px.to_string(), py.to_string()]);
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.xdotool(&args)?;
        self.pointer = to;
        Ok(())
    }

    fn click(&mut self, button: Button, double: bool) -> Result<(), String> {
        let button = match button {
            Button::Left => "1",
            Button::Right => "3",
        };
        let repeat = if double { "2" } else { "1" };
        self.xdotool(&["click", "--repeat", repeat, button])
            .map(drop)
    }

    fn title(&mut self) -> Result<String, String> {
        let id = self.id.clone();
        self.xdotool(&["getwindowname", &id])
    }
}

/// The points a glide passes through, ending on `to`: one every 20ms or
/// so, eased in and out.
pub fn glide(from: (i32, i32), to: (i32, i32), over: Duration) -> Vec<(i32, i32)> {
    let n = (over.as_millis() / 20).clamp(1, 60) as usize;
    (1..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let eased = t * t * (3.0 - 2.0 * t);
            let at = |a: i32, b: i32| a + ((b - a) as f64 * eased).round() as i32;
            (at(from.0, to.0), at(from.1, to.1))
        })
        .collect()
}

/// A chord as xdotool's `key` spells it: `ctrl+shift+space`.
pub fn keysym(chord: &Chord) -> String {
    let mut parts: Vec<String> = chord
        .modifiers
        .iter()
        .map(|m| {
            match m {
                Modifier::Ctrl => "ctrl",
                Modifier::Alt => "alt",
                Modifier::Shift => "shift",
                Modifier::Super => "super",
            }
            .to_string()
        })
        .collect();
    parts.push(match &chord.key {
        Key::Named(name) => match *name {
            "Enter" => "Return".into(),
            "Space" => "space".into(),
            "Backspace" => "BackSpace".into(),
            "PageUp" => "Page_Up".into(),
            "PageDown" => "Page_Down".into(),
            other => other.to_string(),
        },
        Key::Char(c) => char_keysym(*c),
    });
    parts.join("+")
}

/// X's name for a character's key: `?` is `question`. Letters and digits
/// are their own names.
fn char_keysym(c: char) -> String {
    let named = match c {
        ' ' => "space",
        '?' => "question",
        '/' => "slash",
        '\\' => "backslash",
        '.' => "period",
        ',' => "comma",
        '-' => "minus",
        '+' => "plus",
        '=' => "equal",
        ';' => "semicolon",
        ':' => "colon",
        '\'' => "apostrophe",
        '"' => "quotedbl",
        '[' => "bracketleft",
        ']' => "bracketright",
        '!' => "exclam",
        '#' => "numbersign",
        '*' => "asterisk",
        '&' => "ampersand",
        '<' => "less",
        '>' => "greater",
        '`' => "grave",
        _ => return c.to_string(),
    };
    named.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_are_spelled_as_xdotool_reads_them() {
        let chord = |line: &str| match crate::desktop::script::classify(line) {
            Ok(crate::desktop::script::Action::Press { chord, .. }) => chord,
            other => panic!("{line}: {other:?}"),
        };
        assert_eq!(keysym(&chord("Ctrl+Shift+Space")), "ctrl+shift+space");
        assert_eq!(keysym(&chord("Enter")), "Return");
        assert_eq!(keysym(&chord("Ctrl+?")), "ctrl+question");
        assert_eq!(keysym(&chord("Key e")), "e");
        assert_eq!(keysym(&chord("Cmd+PageDown")), "super+Page_Down");
    }

    #[test]
    fn a_glide_ends_where_it_was_sent() {
        let steps = glide((0, 0), (100, 50), Duration::from_millis(400));
        assert_eq!(steps.len(), 20);
        assert_eq!(*steps.last().unwrap(), (100, 50));
        assert!(steps[0].0 < 10, "eased: it starts slowly");
    }
}
