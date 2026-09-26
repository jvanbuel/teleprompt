//! Speech into lines, and each command placed against them.

use crate::tape::{commands, tape};
use crate::{Draft, Options, Trace, Word};

/// A narration line: what was said, as the script will read it, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// How a tape runs against its line: the policy the script will name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Started in the pause after the line: runs after it.
    Hold,
    /// Started while the line was said: runs with it.
    Concurrent,
}

/// A tape, and how it runs against the line above it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub mode: Mode,
    /// The phrase being said when a concurrent tape started, if it started
    /// after the line's first words.
    pub cue: Option<String>,
    pub tape: String,
}

/// A line and the tapes that follow it; no line for tapes typed before
/// anything was said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beat {
    pub line: Option<Line>,
    pub blocks: Vec<Block>,
}

/// A command this far into its line or more is cued to where it started;
/// sooner, it just starts with the line.
const CUE_AFTER_MS: u64 = 600;
/// A command started this soon before a line's first word runs with it.
const EARLY_MS: u64 = 150;

/// The script a session makes: `words` cut into lines at pauses, and each
/// command run with the line it was typed during, or after the line it
/// followed.
pub fn derive(trace: &Trace, words: &[Word], options: &Options) -> Draft {
    let segments = segment(words, options.pause_ms);
    let cmds = commands(trace);

    // Where each command goes: its segment (none before the first) and mode.
    let placed: Vec<(Option<usize>, Mode)> =
        cmds.iter().map(|c| place(c.start(), &segments)).collect();

    let mut beats: Vec<Beat> = Vec::new();
    if placed.iter().any(|(seg, _)| seg.is_none()) {
        beats.push(Beat {
            line: None,
            blocks: Vec::new(),
        });
    }
    beats.extend(segments.iter().map(|s| Beat {
        line: Some(line(s)),
        blocks: Vec::new(),
    }));

    let mut i = 0;
    while i < cmds.len() {
        let group = placed[i];
        let len = placed[i..].iter().take_while(|p| **p == group).count();
        // The session's next keystroke, even one of a command left out.
        let end = cmds[i + len - 1].end();
        let next = trace.input.iter().map(|(t, _)| *t).find(|&t| t > end);
        let (seg, mode) = group;
        let cue = match (seg, mode) {
            (Some(s), Mode::Concurrent) => cue(&segments[s], cmds[i].start()),
            _ => None,
        };
        let beat = match seg {
            Some(s) => s + usize::from(beats[0].line.is_none()),
            None => 0,
        };
        beats[beat].blocks.push(Block {
            mode,
            cue,
            tape: tape(&cmds[i..i + len], &trace.output, next, options.pause_ms),
        });
        i += len;
    }
    Draft { beats }
}

/// Words cut where the speaker paused.
fn segment(words: &[Word], pause_ms: u64) -> Vec<Vec<Word>> {
    let mut out: Vec<Vec<Word>> = Vec::new();
    for w in words {
        match out.last_mut() {
            Some(seg) if w.start_ms < seg[seg.len() - 1].end_ms + pause_ms => seg.push(w.clone()),
            _ => out.push(vec![w.clone()]),
        }
    }
    out
}

fn place(at: u64, segments: &[Vec<Word>]) -> (Option<usize>, Mode) {
    let during = segments
        .iter()
        .position(|s| s[0].start_ms <= at + EARLY_MS && at <= s[s.len() - 1].end_ms);
    match during {
        Some(i) => (Some(i), Mode::Concurrent),
        None => (
            segments.iter().rposition(|s| s[s.len() - 1].end_ms <= at),
            Mode::Hold,
        ),
    }
}

fn line(words: &[Word]) -> Line {
    Line {
        text: sentence(&spoken(words)),
        start_ms: words[0].start_ms,
        end_ms: words[words.len() - 1].end_ms,
    }
}

/// The words as the script writes them. A recognizer that hears in capitals
/// is lowered, with `I` kept; anything else is kept as heard.
fn spoken(words: &[Word]) -> Vec<String> {
    let shouting = words
        .iter()
        .all(|w| !w.text.chars().any(char::is_lowercase));
    words
        .iter()
        .map(|w| {
            if !shouting {
                return w.text.clone();
            }
            let lower = w.text.to_lowercase();
            if lower == "i" || lower.starts_with("i'") {
                capitalized(&lower)
            } else {
                lower
            }
        })
        .collect()
}

/// Words as a sentence: capitalized, and ending in a full stop unless it
/// already ends a sentence. A line cut at a pause after a comma ends in the
/// full stop instead.
fn sentence(words: &[String]) -> String {
    let joined = words.join(" ");
    let mut text = capitalized(joined.trim_end_matches([',', ';', ':']));
    if !text.ends_with(['.', '!', '?']) {
        text.push('.');
    }
    text
}

fn capitalized(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// The phrase being said at `at`, long enough to occur once in the line.
fn cue(words: &[Word], at: u64) -> Option<String> {
    if at < words[0].start_ms + CUE_AFTER_MS {
        return None;
    }
    let k = words.iter().rposition(|w| w.start_ms <= at + 100)?;
    if k == 0 {
        return None;
    }
    let written = spoken(words);
    let text = sentence(&written);
    let offset: usize = written[..k].iter().map(|w| w.len() + 1).sum();
    (k + 2..=written.len())
        .map(|end| written[k..end].join(" "))
        .find(|phrase| text.find(phrase.as_str()) == Some(offset))
}
