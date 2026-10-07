//! The action language a desktop block is written in.
//!
//! It borrows VHS's vocabulary (`Type`, `Sleep`, `Enter`, `Ctrl+O`,
//! `Set TypingSpeed`), so a tape's author can read one, and adds what a
//! window needs and a terminal does not: `Key` for a bare letter, the
//! pointer (`Move`, `Click`), and `Wait` for a window's title.

use teleprompt_scene::core::attrs::parse_duration_ms;
use teleprompt_scene::CommandError;

/// The mark, spelled as a comment (`docs/design.md#marks`).
pub const MARK: &str = "# mark";

/// A key press's cost and a `Type`'s per-character gap, unless set.
pub const DEFAULT_TYPING_SPEED_MS: u64 = 50;

/// How long the pointer takes to glide to a `Move` or `Click`, unless set.
/// Long enough to follow by eye: a pointer that jumps is a click from
/// nowhere.
pub const DEFAULT_POINTER_SPEED_MS: u64 = 400;

/// How long a `Wait` may take, unless set: VHS's own bound.
pub const DEFAULT_WAIT_TIMEOUT_MS: u64 = 15_000;

/// Keys pressed by name. A letter or digit is pressed with `Key`, or after
/// a modifier: a lone `E` on a line is more likely a typo than a key.
pub const KEYS: &[&str] = &[
    "Enter",
    "Tab",
    "Space",
    "Backspace",
    "Delete",
    "Escape",
    "Up",
    "Down",
    "Left",
    "Right",
    "PageUp",
    "PageDown",
    "Home",
    "End",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
];

/// A modifier as written, and the one it means: `Cmd` and `Super` are one
/// key, the one beside the space bar that is neither Alt nor Ctrl.
const MODIFIERS: &[(&str, Modifier)] = &[
    ("Ctrl", Modifier::Ctrl),
    ("Alt", Modifier::Alt),
    ("Shift", Modifier::Shift),
    ("Super", Modifier::Super),
    ("Cmd", Modifier::Super),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Super,
}

/// One key with the modifiers held for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub modifiers: Vec<Modifier>,
    pub key: Key,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// One of [`KEYS`].
    Named(&'static str),
    Char(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// What one line of a block does.
///
/// [`classify`] is the only place a line is read, so validating, timing,
/// re-timing and running cannot disagree about one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Blank, or a comment.
    Nothing,
    Mark,
    Sleep(u64),
    /// `speed` is a `Type@…` override of the typing speed.
    Type {
        text: String,
        speed: Option<u64>,
    },
    Press {
        chord: Chord,
        count: u64,
        speed: Option<u64>,
    },
    /// The pointer to `(x, y)` in the window, and a press there if
    /// `button` is set; `speed` overrides the glide.
    Pointer {
        x: u32,
        y: u32,
        button: Option<Button>,
        double: bool,
        speed: Option<u64>,
    },
    /// Until the window's title contains `text`.
    Wait {
        text: String,
        timeout: Option<u64>,
    },
    TypingSpeed(u64),
    PointerSpeed(u64),
    WaitTimeout(u64),
}

/// The lone place a line is interpreted.
pub fn classify(line: &str) -> Result<Action, CommandError> {
    let line = line.trim();
    if line == MARK {
        return Ok(Action::Mark);
    }
    if line.is_empty() || line.starts_with('#') {
        return Ok(Action::Nothing);
    }
    let (head, rest) = match line.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim()),
        None => (line, ""),
    };
    let (name, at) = match head.split_once('@') {
        Some((name, ms)) => {
            let ms = parse_duration_ms(ms)
                .map_err(|m| CommandError::new(m, format!("e.g. `{name}@100ms`")))?;
            (name, Some(ms))
        }
        None => (head, None),
    };

    match name {
        "Sleep" => {
            refuse_at(name, at)?;
            one_duration(name, rest).map(Action::Sleep)
        }
        "Type" => {
            let (text, tail) = quoted(name, rest)?;
            nothing_after(name, tail)?;
            Ok(Action::Type { text, speed: at })
        }
        "Key" => {
            let (chord, count) = chord_and_count(rest, true)?;
            Ok(Action::Press {
                chord,
                count,
                speed: at,
            })
        }
        "Move" | "Click" | "DoubleClick" | "RightClick" => pointer(name, rest, at),
        "Wait" => {
            let (text, tail) = quoted(name, rest)?;
            nothing_after(name, tail)?;
            Ok(Action::Wait { text, timeout: at })
        }
        "Set" => {
            refuse_at(name, at)?;
            setting(rest)
        }
        _ => {
            if let Ok(chord) = parse_chord(name, false) {
                let (_, count) = chord_and_count(&format!("{name} {rest}"), false)?;
                return Ok(Action::Press {
                    chord,
                    count,
                    speed: at,
                });
            }
            Err(unknown(name))
        }
    }
}

fn unknown(name: &str) -> CommandError {
    let commands = [
        "Type",
        "Key",
        "Sleep",
        "Move",
        "Click",
        "DoubleClick",
        "RightClick",
        "Wait",
        "Set",
    ];
    let help = match commands
        .iter()
        .chain(KEYS)
        .find(|c| c.eq_ignore_ascii_case(name))
    {
        Some(c) => format!("commands are capitalised: write `{c}`"),
        None if name.chars().count() == 1 => format!("press a single key with `Key {name}`"),
        None => format!(
            "a line is one of {}, or a key such as `Enter` or `Ctrl+O`",
            commands.join(", ")
        ),
    };
    CommandError::new(format!("unknown command `{name}`"), help)
}

fn refuse_at(name: &str, at: Option<u64>) -> Result<(), CommandError> {
    match at {
        Some(_) => Err(CommandError::new(
            format!("`{name}` takes no `@` speed"),
            format!("write `{name}` without it"),
        )),
        None => Ok(()),
    }
}

fn one_duration(name: &str, rest: &str) -> Result<u64, CommandError> {
    let help = format!("e.g. `{name} 2s`");
    let mut words = rest.split_whitespace();
    let (Some(value), None) = (words.next(), words.next()) else {
        return Err(CommandError::new(
            format!("`{name}` takes one duration"),
            help,
        ));
    };
    parse_duration_ms(value).map_err(|m| CommandError::new(m, help))
}

fn setting(rest: &str) -> Result<Action, CommandError> {
    let (name, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let ms = |make: fn(u64) -> Action| one_duration(&format!("Set {name}"), value.trim()).map(make);
    match name {
        "TypingSpeed" => ms(Action::TypingSpeed),
        "PointerSpeed" => ms(Action::PointerSpeed),
        "WaitTimeout" => ms(Action::WaitTimeout),
        other => Err(CommandError::new(
            format!("unknown setting `{other}`"),
            "a block sets TypingSpeed, PointerSpeed or WaitTimeout; the window's size and \
             the app are the scene's settings",
        )),
    }
}

/// `Click 120 40`: a point in the window, from its top-left corner.
fn pointer(name: &str, rest: &str, at: Option<u64>) -> Result<Action, CommandError> {
    let help = format!("e.g. `{name} 120 40`, measured from the window's top-left corner");
    let mut words = rest.split_whitespace();
    let (Some(x), Some(y), None) = (words.next(), words.next(), words.next()) else {
        return Err(CommandError::new(
            format!("`{name}` takes a point: two numbers"),
            help,
        ));
    };
    let (Ok(x), Ok(y)) = (x.parse::<u32>(), y.parse::<u32>()) else {
        return Err(CommandError::new(
            format!("`{name}`'s point must be two whole numbers, found `{rest}`"),
            help,
        ));
    };
    let (button, double) = match name {
        "Move" => (None, false),
        "Click" => (Some(Button::Left), false),
        "DoubleClick" => (Some(Button::Left), true),
        _ => (Some(Button::Right), false),
    };
    Ok(Action::Pointer {
        x,
        y,
        button,
        double,
        speed: at,
    })
}

/// `Ctrl+Shift+Space 2`: a chord and how many times to press it.
fn chord_and_count(rest: &str, after_key: bool) -> Result<(Chord, u64), CommandError> {
    let mut words = rest.split_whitespace();
    let Some(written) = words.next() else {
        return Err(CommandError::new(
            "`Key` needs a key",
            "e.g. `Key e` or `Key Ctrl+O`",
        ));
    };
    let chord = parse_chord(written, after_key)?;
    let count = match words.next() {
        None => 1,
        Some(n) => n.parse::<u64>().ok().filter(|n| *n > 0).ok_or_else(|| {
            CommandError::new(
                format!("`{written}` takes an optional repeat count, found `{n}`"),
                format!("e.g. `{written} 3`"),
            )
        })?,
    };
    if let Some(extra) = words.next() {
        return Err(CommandError::new(
            format!("`{written}` has trailing `{extra}`"),
            "one key per line",
        ));
    }
    Ok((chord, count))
}

/// A key, with modifiers joined by `+`. A lone character is a key only
/// after `Key` or a modifier.
fn parse_chord(written: &str, after_key: bool) -> Result<Chord, CommandError> {
    let bad = || {
        CommandError::new(
            format!("`{written}` is not a key"),
            format!(
                "a key is a character or one of {}, after modifiers such as `Ctrl+`",
                KEYS.join(", ")
            ),
        )
    };
    // `+` is the separator and a key: `Key +` and `Ctrl++` press it.
    let (held, last) = match written.strip_suffix('+') {
        Some("") => ("", "+"),
        Some(before) if before.ends_with('+') => (&before[..before.len() - 1], "+"),
        _ => written.rsplit_once('+').unwrap_or(("", written)),
    };
    let held: Vec<&str> = if held.is_empty() {
        Vec::new()
    } else {
        held.split('+').collect()
    };
    if last.is_empty() {
        return Err(bad());
    }
    let mut modifiers = Vec::new();
    for part in &held {
        let (_, m) = MODIFIERS.iter().find(|(n, _)| n == part).ok_or_else(bad)?;
        modifiers.push(*m);
    }
    let key = if let Some(named) = KEYS.iter().find(|k| **k == last) {
        Key::Named(named)
    } else {
        let mut chars = last.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if after_key || !modifiers.is_empty() => Key::Char(c),
            _ => return Err(bad()),
        }
    };
    Ok(Chord { modifiers, key })
}

fn nothing_after(name: &str, tail: &str) -> Result<(), CommandError> {
    if tail.trim().is_empty() {
        Ok(())
    } else {
        Err(CommandError::new(
            format!("`{name}` has trailing `{}`", tail.trim()),
            "one command per line",
        ))
    }
}

/// One quoted string, `"…"`, `'…'` or `` `…` ``, and the rest of the line.
fn quoted<'a>(name: &str, rest: &'a str) -> Result<(String, &'a str), CommandError> {
    let help = format!("e.g. `{name} \"Open script\"`");
    let mut chars = rest.char_indices();
    let open = match chars.next() {
        Some((_, c @ ('"' | '\'' | '`'))) => c,
        _ => {
            return Err(CommandError::new(
                format!("`{name}` needs a quoted string"),
                help,
            ))
        }
    };
    let mut out = String::new();
    let mut escaped = false;
    for (i, c) in chars {
        if escaped {
            match c {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                c if c == open || c == '\\' => out.push(c),
                c => {
                    out.push('\\');
                    out.push(c);
                }
            }
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == open {
            return Ok((out, &rest[i + c.len_utf8()..]));
        } else {
            out.push(c);
        }
    }
    Err(CommandError::new(
        format!("`{name}`'s string is never closed"),
        help,
    ))
}

/// How long a sequence of lines is on screen: `(ms, waits)`, where a
/// `Wait` counts at its timeout and makes the total a bound.
pub fn timing<'a>(lines: impl IntoIterator<Item = &'a str>) -> (u64, bool) {
    let mut pace = Pace::default();
    let mut total: u64 = 0;
    let mut waited = false;
    for line in lines {
        let Ok(action) = classify(line) else { continue };
        if let Action::Wait { .. } = action {
            waited = true;
        }
        total = total.saturating_add(pace.cost(&action));
    }
    (total, waited)
}

/// The speeds in force at a point in a block.
#[derive(Debug, Clone, Copy)]
pub struct Pace {
    pub typing: u64,
    pub pointer: u64,
    pub wait: u64,
}

impl Default for Pace {
    fn default() -> Self {
        Pace {
            typing: DEFAULT_TYPING_SPEED_MS,
            pointer: DEFAULT_POINTER_SPEED_MS,
            wait: DEFAULT_WAIT_TIMEOUT_MS,
        }
    }
}

impl Pace {
    /// What `action` costs on screen, taking in any speed it sets.
    pub fn cost(&mut self, action: &Action) -> u64 {
        match action {
            Action::Sleep(ms) => *ms,
            Action::Type { text, speed } => {
                let chars = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
                chars.saturating_mul(speed.unwrap_or(self.typing))
            }
            Action::Press { count, speed, .. } => {
                count.saturating_mul(speed.unwrap_or(self.typing))
            }
            Action::Pointer { speed, .. } => speed.unwrap_or(self.pointer),
            Action::Wait { timeout, .. } => timeout.unwrap_or(self.wait),
            Action::TypingSpeed(ms) => {
                self.typing = *ms;
                0
            }
            Action::PointerSpeed(ms) => {
                self.pointer = *ms;
                0
            }
            Action::WaitTimeout(ms) => {
                self.wait = *ms;
                0
            }
            Action::Nothing | Action::Mark => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(line: &str) -> Action {
        classify(line).unwrap_or_else(|e| panic!("{line}: {}", e.message))
    }

    fn err(line: &str) -> CommandError {
        classify(line).expect_err(line)
    }

    #[test]
    fn keys_are_named_chorded_or_pressed_with_key() {
        assert_eq!(
            ok("Ctrl+Shift+Space"),
            Action::Press {
                chord: Chord {
                    modifiers: vec![Modifier::Ctrl, Modifier::Shift],
                    key: Key::Named("Space")
                },
                count: 1,
                speed: None
            }
        );
        assert_eq!(
            ok("Key e"),
            Action::Press {
                chord: Chord {
                    modifiers: vec![],
                    key: Key::Char('e')
                },
                count: 1,
                speed: None
            }
        );
        assert!(matches!(ok("Down 3"), Action::Press { count: 3, .. }));
        assert!(matches!(
            ok("Key + 2"),
            Action::Press {
                chord: Chord {
                    key: Key::Char('+'),
                    ..
                },
                count: 2,
                ..
            }
        ));
        assert!(
            matches!(ok("Ctrl++"), Action::Press { chord: Chord { key: Key::Char('+'), ref modifiers }, .. } if modifiers == &[Modifier::Ctrl])
        );
        assert!(err("Key Ctrl+").message.contains("not a key"));
        assert!(err("Key Ctrl++Shift+a").message.contains("not a key"));
        assert!(
            matches!(ok("Cmd+O"), Action::Press { chord: Chord { ref modifiers, .. }, .. } if modifiers == &[Modifier::Super])
        );
    }

    /// A lone letter is a typo more often than a key, so it says how to
    /// press one.
    #[test]
    fn a_lone_letter_asks_for_key() {
        let e = err("E");
        assert!(e.help.unwrap().contains("`Key E`"));
        assert!(err("enter").help.unwrap().contains("`Enter`"));
    }

    #[test]
    fn the_pointer_takes_a_point_in_the_window() {
        assert_eq!(
            ok("Click@600ms 120 40"),
            Action::Pointer {
                x: 120,
                y: 40,
                button: Some(Button::Left),
                double: false,
                speed: Some(600)
            }
        );
        assert!(matches!(
            ok("Move 1 2"),
            Action::Pointer { button: None, .. }
        ));
        assert!(err("Click 120").message.contains("two numbers"));
        assert!(err("Click -1 4").message.contains("whole numbers"));
    }

    #[test]
    fn type_and_wait_take_quoted_text() {
        assert_eq!(
            ok(r#"Type "say \"hi\"""#),
            Action::Type {
                text: "say \"hi\"".into(),
                speed: None
            }
        );
        assert_eq!(
            ok("Wait@5s 'tour.md'"),
            Action::Wait {
                text: "tour.md".into(),
                timeout: Some(5000)
            }
        );
        assert!(err("Type hello").message.contains("quoted"));
        assert!(err(r#"Type "a" b"#).message.contains("trailing"));
    }

    #[test]
    fn only_timing_settings_belong_in_a_block() {
        assert_eq!(ok("Set TypingSpeed 80ms"), Action::TypingSpeed(80));
        let e = err("Set Width 800");
        assert!(
            e.help.unwrap().contains("scene's settings"),
            "the window is the scene's"
        );
    }

    #[test]
    fn a_blocks_timing_is_the_sum_of_what_it_does_at_its_speeds() {
        let lines = [
            "Type \"abc\"", // 3 × 50
            "Set TypingSpeed 100ms",
            "Enter 2",     // 2 × 100
            "Click 10 10", // 400
            "Sleep 1s",
        ];
        assert_eq!(timing(lines), (150 + 200 + 400 + 1000, false));
        assert_eq!(timing(["Wait@2s \"x\""]), (2000, true));
    }
}
