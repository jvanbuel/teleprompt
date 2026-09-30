//! Captions for a video: each narration line as subtitle cues, written as
//! SRT or WebVTT.
//!
//! A cue holds at most two rows of [`ROW`] characters, the usual limit for
//! subtitles read at speaking pace. A line longer than that is split,
//! after a sentence or a comma where one falls far enough in, and each part
//! is shown from when its first word is said: by the line's word timings
//! when the voice gave them, or by its share of the text otherwise.

use crate::{LineEntry, NarrationManifest};

/// Characters in a row of a cue.
pub const ROW: usize = 42;

/// How far into a cue a sentence or clause must end for the cue to be cut
/// there rather than filled, as a share of two rows.
const EARLIEST_BREAK: f64 = 0.4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub rows: Vec<String>,
    /// Who says it, from the script's cast; the narrator when `None`.
    pub speaker: Option<String>,
}

/// Every line of `manifest`, as cues in order.
pub fn cues(manifest: &NarrationManifest) -> Vec<Cue> {
    manifest.lines.iter().flat_map(line_cues).collect()
}

fn line_cues(line: &LineEntry) -> Vec<Cue> {
    let words: Vec<&str> = line.text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let starts = word_starts(line, &words);
    let end = (line.start_ms + line.duration_ms).ms();
    let parts = split(&words);
    parts
        .iter()
        .enumerate()
        .map(|(i, &(from, to))| Cue {
            start_ms: starts[from],
            end_ms: parts.get(i + 1).map_or(end, |&(next, _)| starts[next]),
            rows: rows(&words[from..to]),
            speaker: line.speaker.clone(),
        })
        .collect()
}

/// When each word starts, in the video: from the voice's timings when they
/// are one per word, or spread by characters over the line otherwise.
fn word_starts(line: &LineEntry, words: &[&str]) -> Vec<u64> {
    if let Some(timed) = line.words.as_ref().filter(|t| t.len() == words.len()) {
        return timed
            .iter()
            .map(|w| line.start_ms.ms() + w.start_ms)
            .collect();
    }
    let total: usize = words.iter().map(|w| width(w) + 1).sum();
    let mut before = 0;
    words
        .iter()
        .map(|w| {
            let at = line.start_ms.ms() + line.duration_ms.ms() * before as u64 / total as u64;
            before += width(w) + 1;
            at
        })
        .collect()
}

/// The words cut into cues, as `(first, past_last)` indices.
fn split(words: &[&str]) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let mut from = 0;
    while from < words.len() {
        let mut to = from + 1;
        while to < words.len() && fits(&words[from..=to]) {
            to += 1;
        }
        if to < words.len() {
            to = natural_break(words, from, to).unwrap_or(to);
        }
        parts.push((from, to));
        from = to;
    }
    parts
}

/// The last sentence end, or failing that the last comma, in
/// `words[from..to]` that falls far enough in to end a cue.
fn natural_break(words: &[&str], from: usize, to: usize) -> Option<usize> {
    let earliest = (EARLIEST_BREAK * (2 * ROW) as f64) as usize;
    let long_enough = |end: usize| width(&words[from..end].join(" ")) >= earliest;
    let last = |marks: &[char]| {
        (from + 1..to)
            .rev()
            .find(|&end| words[end - 1].ends_with(marks) && long_enough(end))
    };
    last(&['.', '!', '?']).or_else(|| last(&[',', ';', ':']))
}

fn fits(words: &[&str]) -> bool {
    width(&words.join(" ")) <= ROW
        || (1..words.len()).any(|i| halves(words, i).iter().all(|r| width(r) <= ROW))
}

/// The words as one row, or two as even as they will go.
fn rows(words: &[&str]) -> Vec<String> {
    let text = words.join(" ");
    if width(&text) <= ROW || words.len() == 1 {
        return vec![text];
    }
    let best = (1..words.len())
        .min_by_key(|&i| {
            let [a, b] = halves(words, i);
            width(&a).max(width(&b))
        })
        .unwrap_or(1);
    halves(words, best).to_vec()
}

/// A row's length as a reader sees it.
fn width(s: &str) -> usize {
    s.chars().count()
}

fn halves(words: &[&str], at: usize) -> [String; 2] {
    [words[..at].join(" "), words[at..].join(" ")]
}

/// The cues as SubRip, numbered from one.
pub fn srt(cues: &[Cue]) -> String {
    let blocks: Vec<String> = cues
        .iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                "{}\n{} --> {}\n{}\n",
                i + 1,
                clock(c.start_ms, ','),
                clock(c.end_ms, ','),
                c.rows.join("\n")
            )
        })
        .collect();
    blocks.join("\n")
}

/// The cues as WebVTT, a speaker's cue in their voice span (`<v guest>`)
/// so a player can say who is talking.
pub fn vtt(cues: &[Cue]) -> String {
    let blocks: Vec<String> = cues
        .iter()
        .map(|c| {
            let voice = c
                .speaker
                .as_ref()
                .map_or(String::new(), |s| format!("<v {s}>"));
            format!(
                "{} --> {}\n{voice}{}\n",
                clock(c.start_ms, '.'),
                clock(c.end_ms, '.'),
                c.rows.join("\n")
            )
        })
        .collect();
    format!("WEBVTT\n\n{}", blocks.join("\n"))
}

/// `hh:mm:ss<sep>mmm`.
fn clock(ms: u64, sep: char) -> String {
    let (h, m, s) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60);
    format!("{h:02}:{m:02}:{s:02}{sep}{:03}", ms % 1000)
}
