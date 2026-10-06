//! Speech into lines, and each step placed against them.

use std::ops::Range;

use crate::derive::{Draft, Options, Word};

/// A narration line: what was said, as the script will read it, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// How a block runs against its line: the policy the script will name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Started in the pause after the line: runs after it.
    Hold,
    /// Started while the line was said: runs with it.
    Concurrent,
}

/// A run of the recording's steps, and how it runs against the line above
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub mode: Mode,
    /// The phrase being said when a concurrent block started, if it
    /// started after the line's first words.
    pub cue: Option<String>,
    pub steps: Range<usize>,
}

/// A line and the blocks that follow it; no line for steps taken before
/// anything was said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beat {
    pub line: Option<Line>,
    pub blocks: Vec<Block>,
}

/// A step this far into its line or more is cued to where it started;
/// sooner, it just starts with the line.
const CUE_AFTER_MS: u64 = 600;
/// A step started this soon before a line's first word runs with it.
const EARLY_MS: u64 = 150;

/// The script a session makes: `words` cut into lines at pauses, and each
/// step, by when it began (`starts`, in order), run with the line it was
/// taken during, or after the line it followed. Steps placed alike in a
/// row are one block.
pub fn derive(starts: &[u64], words: &[Word], options: &Options) -> Draft {
    let segments = segment(words, options.pause_ms);

    // Where each step goes: its segment (none before the first) and mode.
    let placed: Vec<(Option<usize>, Mode)> =
        starts.iter().map(|&at| place(at, &segments)).collect();

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
    while i < starts.len() {
        let group = placed[i];
        let len = placed[i..].iter().take_while(|p| **p == group).count();
        let (seg, mode) = group;
        let cue = match (seg, mode) {
            (Some(s), Mode::Concurrent) => cue(&segments[s], starts[i]),
            _ => None,
        };
        let beat = match seg {
            Some(s) => s + usize::from(beats[0].line.is_none()),
            None => 0,
        };
        beats[beat].blocks.push(Block {
            mode,
            cue,
            steps: i..i + len,
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
