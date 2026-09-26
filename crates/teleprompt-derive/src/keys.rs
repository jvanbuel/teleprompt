//! Keystrokes as a terminal receives them, decoded into the keys a tape
//! presses.

/// One key, spelled as a VHS tape spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    /// `Enter`, `Tab`, `Up`, …
    Named(&'static str),
    Ctrl(char),
    Alt(char),
}

impl Key {
    /// The tape command that presses it; `Char` is typed, not pressed.
    pub fn command(&self) -> Option<String> {
        match self {
            Key::Char(_) => None,
            Key::Named(name) => Some((*name).to_string()),
            Key::Ctrl(c) => Some(format!("Ctrl+{c}")),
            Key::Alt(c) => Some(format!("Alt+{c}")),
        }
    }
}

/// CSI and SS3 finals, and the `~` codes, that are keys.
const SEQUENCES: &[(&str, &str)] = &[
    ("[A", "Up"),
    ("[B", "Down"),
    ("[C", "Right"),
    ("[D", "Left"),
    ("OA", "Up"),
    ("OB", "Down"),
    ("OC", "Right"),
    ("OD", "Left"),
    ("[H", "Home"),
    ("[F", "End"),
    ("OH", "Home"),
    ("OF", "End"),
    ("[2~", "Insert"),
    ("[3~", "Delete"),
    ("[5~", "PageUp"),
    ("[6~", "PageDown"),
];

/// The keys in what a terminal read from its keyboard. A paste is its
/// characters; an escape sequence that is no key is dropped.
pub fn decode(input: &str) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut rest = input;
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        match c {
            '\r' | '\n' => keys.push(Key::Named("Enter")),
            '\t' => keys.push(Key::Named("Tab")),
            '\x7f' | '\x08' => keys.push(Key::Named("Backspace")),
            '\x1b' => rest = escape(rest, &mut keys),
            '\x01'..='\x1a' => keys.push(Key::Ctrl((b'A' + c as u8 - 1) as char)),
            c if c.is_control() => {}
            c => keys.push(Key::Char(c)),
        }
    }
    keys
}

/// Reads what follows an ESC, pushing the key it makes, and returns the
/// rest of the input.
fn escape<'a>(rest: &'a str, keys: &mut Vec<Key>) -> &'a str {
    if let Some((seq, key)) = SEQUENCES.iter().find(|(seq, _)| rest.starts_with(seq)) {
        keys.push(Key::Named(key));
        return &rest[seq.len()..];
    }
    if let Some(body) = rest.strip_prefix('[') {
        // Some other CSI sequence: parameters, then one final byte.
        let end = body
            .find(|c: char| ('@'..='~').contains(&c))
            .map_or(body.len(), |i| i + 1);
        return &body[end..];
    }
    match rest.chars().next() {
        Some(c) if c.is_ascii_alphanumeric() => {
            keys.push(Key::Alt(c));
            &rest[1..]
        }
        _ => {
            keys.push(Key::Named("Escape"));
            rest
        }
    }
}
