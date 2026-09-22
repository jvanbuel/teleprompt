//! A tape as a sequence of things to do to a terminal.
//!
//! The parsing is [`teleprompt_scene_vhs::classify`]'s — the same function
//! the compiler measures a tape with, deliberately. Two parsers for one
//! language is a tape that `check` accepts and nothing can run, and the
//! difference would show up as a video of the wrong length rather than as
//! an error.
//!
//! What this adds is the other half of the question. The compiler asks
//! *how long does this line take*; a capture asks *what does it send*.

use teleprompt_scene_vhs::{classify, Line, Setting};

/// VHS's default, and the one the compiler assumes when a tape is silent.
pub const DEFAULT_TYPING_MS: u64 = 50;
/// VHS's default `Set WaitTimeout`.
pub const DEFAULT_WAIT_MS: u64 = 5_000;

/// One thing to do to the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Send `text`, one character every `per_char_ms`.
    Type {
        text: String,
        per_char_ms: u64,
    },
    /// Send `bytes` `count` times, `per_key_ms` apart.
    Keys {
        bytes: Vec<u8>,
        count: u64,
        per_key_ms: u64,
    },
    /// Wait, doing nothing.
    Sleep(u64),
    /// Wait until the program stops writing, or `timeout_ms` passes.
    ///
    /// VHS waits for a shell prompt to match a pattern. Waiting for the
    /// screen to go still is not the same rule, and it is the one that can
    /// be applied without knowing what the prompt looks like — which
    /// teleprompt does not, because it does not own the shell's `PS1` any
    /// more than the tape does.
    Quiet {
        timeout_ms: u64,
    },
    /// The span boundary. The tape keeps running; the recording is cut.
    Mark,
    /// Stop recording. What follows runs and is not watched running — a
    /// `cd`, an `export`, a `clear` — and the recording closes over it as
    /// if it had always been that way.
    Hide,
    Show,
    /// Refuse to record unless `program` is there. A tape that needs a
    /// tool and does not have it records a video of `command not found`,
    /// which is worse than an error because it looks like a video.
    Require(String),
    /// `Copy "text"` holds it; `Paste` sends it.
    Copy(String),
    Paste,
}

/// What a tape asks to be sent, in order.
///
/// Errors are the compiler's to report: `check` has already run by the
/// time anything is captured, so a line that does not parse here is a bug
/// in this crate rather than in the script, and it is dropped rather than
/// allowed to stop a recording that is otherwise fine.
pub fn steps(source: &str) -> Vec<Step> {
    let mut typing = DEFAULT_TYPING_MS;
    let mut timeout = DEFAULT_WAIT_MS;
    let mut out = Vec::new();

    for line in source.lines() {
        match classify(line) {
            Ok(Line::Setting(Setting::TypingSpeed(ms))) => typing = ms,
            Ok(Line::Setting(Setting::WaitTimeout(ms))) => timeout = ms,
            Ok(Line::Sleep(ms)) => out.push(Step::Sleep(ms)),
            Ok(Line::Mark) => out.push(Step::Mark),
            Ok(Line::Type { text, speed }) => out.push(Step::Type {
                text,
                per_char_ms: speed.unwrap_or(typing),
            }),
            Ok(Line::Keys { key, count, speed }) => {
                if let Some(bytes) = key_bytes(&key) {
                    out.push(Step::Keys {
                        bytes,
                        count,
                        per_key_ms: speed.unwrap_or(typing),
                    });
                }
            }
            Ok(Line::Wait { timeout: at }) => out.push(Step::Quiet {
                timeout_ms: at.unwrap_or(timeout),
            }),
            Ok(Line::Hide) => out.push(Step::Hide),
            Ok(Line::Show) => out.push(Step::Show),
            Ok(Line::Require(program)) => out.push(Step::Require(program)),
            Ok(Line::Copy(text)) => out.push(Step::Copy(text)),
            Ok(Line::Paste) => out.push(Step::Paste),
            Ok(Line::Nothing) | Ok(Line::Setting(Setting::Cosmetic)) | Err(_) => {}
        }
    }
    out
}

/// What a key name sends.
///
/// `None` for a name this build cannot send, which is a key the compiler
/// accepted and this did not — the one place the two halves can disagree,
/// and it is why it returns an `Option` rather than guessing.
fn key_bytes(name: &str) -> Option<Vec<u8>> {
    if let Some(rest) = name.strip_prefix("Ctrl+") {
        let c = rest.chars().next()?;
        if !c.is_ascii_alphabetic() || rest.chars().count() != 1 {
            return None;
        }
        // Ctrl+A is 0x01: the letter's position in the alphabet.
        return Some(vec![c.to_ascii_uppercase() as u8 - b'A' + 1]);
    }
    if let Some(rest) = name.strip_prefix("Alt+") {
        let c = rest.chars().next()?;
        if rest.chars().count() != 1 {
            return None;
        }
        let mut bytes = vec![0x1b];
        bytes.extend(c.to_string().into_bytes());
        return Some(bytes);
    }
    if let Some(rest) = name.strip_prefix("Shift+") {
        let c = rest.chars().next()?;
        if rest.chars().count() != 1 {
            return None;
        }
        return Some(c.to_uppercase().to_string().into_bytes());
    }
    let literal = match name {
        "Enter" => "\r",
        "Tab" => "\t",
        "Space" => " ",
        "Backspace" => "\x7f",
        "Delete" => "\x1b[3~",
        "Escape" => "\x1b",
        "Up" => "\x1b[A",
        "Down" => "\x1b[B",
        "Right" => "\x1b[C",
        "Left" => "\x1b[D",
        "Home" => "\x1b[H",
        "End" => "\x1b[F",
        "PageUp" => "\x1b[5~",
        "PageDown" => "\x1b[6~",
        "Insert" => "\x1b[2~",
        _ => return None,
    };
    Some(literal.as_bytes().to_vec())
}

/// The programs a session's tapes say they need and this machine does not
/// have.
///
/// Checked before the terminal opens. A tape that needs a tool it cannot
/// find records a video of `command not found`, which is worse than an
/// error because it looks like a video — it renders, it is the right
/// length, and nothing reports a problem.
pub fn missing(spans: &[Vec<Step>], path: &str) -> Vec<String> {
    let mut out = Vec::new();
    for step in spans.iter().flatten() {
        let Step::Require(program) = step else {
            continue;
        };
        let found = path.split(':').filter(|d| !d.is_empty()).any(|dir| {
            std::path::Path::new(dir)
                .join(program)
                .metadata()
                .is_ok_and(|m| m.is_file())
        });
        if !found && !out.iter().any(|p| p == program) {
            out.push(program.clone());
        }
    }
    out
}
