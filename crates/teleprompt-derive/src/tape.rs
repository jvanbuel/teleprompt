//! Keystrokes into commands, and commands into a VHS tape.

use crate::keys::{decode, Key};
use crate::Trace;

/// The keys up to and including one Enter (or Ctrl+C, Ctrl+D), with when
/// each was pressed.
#[derive(Debug, Clone)]
pub(crate) struct Command {
    pub keys: Vec<(u64, Key)>,
}

impl Command {
    pub fn start(&self) -> u64 {
        self.keys[0].0
    }

    pub fn end(&self) -> u64 {
        self.keys[self.keys.len() - 1].0
    }

    fn typed(&self) -> String {
        self.keys
            .iter()
            .filter_map(|(_, k)| match k {
                Key::Char(c) => Some(*c),
                _ => None,
            })
            .collect()
    }

    /// The command that ended the recording, which is not part of it.
    fn closes_the_shell(&self) -> bool {
        matches!(self.typed().trim(), "exit" | "logout")
            || self.keys.iter().all(|(_, k)| *k == Key::Ctrl('D'))
    }
}

/// The session's keystrokes, cut into commands.
pub(crate) fn commands(trace: &Trace) -> Vec<Command> {
    let mut out = Vec::new();
    let mut keys = Vec::new();
    for (at, input) in &trace.input {
        for key in decode(input) {
            keys.push((*at, key));
            if matches!(key, Key::Named("Enter") | Key::Ctrl('C') | Key::Ctrl('D')) {
                out.push(Command {
                    keys: std::mem::take(&mut keys),
                });
            }
        }
    }
    if !keys.is_empty() {
        out.push(Command { keys });
    }
    if out.last().is_some_and(Command::closes_the_shell) {
        out.pop();
    }
    out
}

/// How long a tape waits for a command's output to finish, past the last
/// thing the terminal wrote.
const SETTLE_MS: u64 = 300;

/// The tape that replays `cmds`, waiting after the last until its output
/// settled: the last output before `next` (the session's next keystroke).
pub(crate) fn tape(cmds: &[Command], output: &[u64], next: Option<u64>, pause_ms: u64) -> String {
    let speed = typing_speed(cmds, pause_ms);
    let mut tape = Tape {
        lines: vec![format!("Set TypingSpeed {speed}ms")],
        typing: String::new(),
    };
    for (i, cmd) in cmds.iter().enumerate() {
        let mut last: Option<u64> = None;
        for &(at, key) in &cmd.keys {
            if let Some(prev) = last.filter(|&p| at - p >= pause_ms) {
                tape.sleep(round((at - prev).saturating_sub(speed)));
            }
            tape.press(key);
            last = Some(at);
        }
        match cmds.get(i + 1) {
            Some(following) => {
                tape.sleep(round((following.start() - cmd.end()).saturating_sub(speed)));
            }
            None => {
                let settled = output
                    .iter()
                    .filter(|&&t| t >= cmd.end() && next.is_none_or(|n| t < n))
                    .max()
                    .map_or(0, |t| t - cmd.end());
                tape.sleep((settled + SETTLE_MS).div_ceil(100) * 100);
            }
        }
    }
    tape.lines.join("\n") + "\n"
}

struct Tape {
    lines: Vec<String>,
    /// Characters not yet written as a `Type`.
    typing: String,
}

impl Tape {
    fn press(&mut self, key: Key) {
        match key {
            Key::Char(c) => self.typing.push(c),
            Key::Named("Backspace") if !self.typing.is_empty() => {
                self.typing.pop();
            }
            _ => {
                self.flush();
                let command = key.command().expect("only characters are typed");
                let repeated = self.lines.last_mut().and_then(|l| {
                    let count = match l.strip_prefix(&command)? {
                        "" => 1,
                        n => n.strip_prefix(' ')?.parse::<u64>().ok()?,
                    };
                    Some((l, count))
                });
                match repeated {
                    Some((line, count)) => *line = format!("{command} {}", count + 1),
                    None => self.lines.push(command),
                }
            }
        }
    }

    fn sleep(&mut self, ms: u64) {
        self.flush();
        if ms > 0 {
            self.lines.push(format!("Sleep {ms}ms"));
        }
    }

    fn flush(&mut self) {
        if !self.typing.is_empty() {
            let text = std::mem::take(&mut self.typing);
            self.lines.push(format!("Type {}", quote(&text)));
        }
    }
}

/// `text` quoted so VHS reads it back: in a delimiter it does not contain
/// where there is one, with backslashes (and the delimiter, if it must)
/// escaped.
fn quote(text: &str) -> String {
    let open = ['"', '\'', '`']
        .into_iter()
        .find(|q| !text.contains(*q))
        .unwrap_or('"');
    let mut out = String::from(open);
    for c in text.chars() {
        if c == '\\' || c == open {
            out.push('\\');
        }
        out.push(c);
    }
    out.push(open);
    out
}

/// The typical gap between two characters typed one after the other, to
/// the nearest 5 ms; VHS's own 50 ms when nothing was typed.
fn typing_speed(cmds: &[Command], pause_ms: u64) -> u64 {
    let mut gaps: Vec<u64> = cmds
        .iter()
        .flat_map(|c| c.keys.windows(2))
        .filter(|w| matches!((w[0].1, w[1].1), (Key::Char(_), Key::Char(_))))
        .map(|w| w[1].0 - w[0].0)
        .filter(|&g| g < pause_ms)
        .collect();
    if gaps.is_empty() {
        return 50;
    }
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2];
    ((median + 2) / 5 * 5).clamp(10, 300)
}

/// A pause, to the nearest 100 ms.
fn round(ms: u64) -> u64 {
    (ms + 50) / 100 * 100
}
