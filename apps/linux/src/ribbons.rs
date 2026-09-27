//! The glass as the timeline: each shot as a ribbon under the words it
//! plays over, or in the pause after its line, and what a drop there asks
//! of the script (`crate::timeline`'s edits).
//!
//! A word is said at its share of its line, by its letters, as the
//! compiler places a cue when the voice gave no word timings.

use std::ops::Range;

use crate::timeline::{Edit, ShotSpan, Timeline};

/// Where a shot lies on the glass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ribbon {
    /// Its index in the timeline's shots.
    pub shot: usize,
    /// The script line it plays with or after.
    pub line: usize,
    /// The words it plays over; empty for a shot held after its line.
    pub words: Range<usize>,
    /// How long it runs past its line's end, in the pause after it.
    pub past_ms: u64,
}

impl Ribbon {
    /// Held after its line: under no words, all in the pause.
    pub fn held(&self) -> bool {
        self.words.is_empty()
    }
}

/// Each word's start and end within its line, in milliseconds from the
/// line's start.
fn shares(text: &str, length_ms: u64) -> Vec<(u64, u64)> {
    let weights: Vec<u64> = text
        .split_whitespace()
        .map(|w| w.chars().count() as u64 + 1)
        .collect();
    let total = weights.iter().sum::<u64>().max(1);
    let mut at = 0;
    weights
        .iter()
        .map(|w| {
            let start = at * length_ms / total;
            at += w;
            (start, at * length_ms / total)
        })
        .collect()
}

/// The script line `id`, and when it is said.
fn span<'a>(
    timeline: &Timeline,
    lines: &'a [(String, String)],
    id: &str,
) -> Option<(usize, &'a str, u64, u64)> {
    let (i, (_, text)) = lines.iter().enumerate().find(|(_, (l, _))| l == id)?;
    let s = timeline.lines.iter().find(|l| l.id == id)?;
    Some((i, text.as_str(), s.start_ms, s.end_ms))
}

/// Every shot that goes with a line, on the glass. `lines` are the
/// script's, as ids and texts.
pub fn ribbons(timeline: &Timeline, lines: &[(String, String)]) -> Vec<Ribbon> {
    timeline
        .shots
        .iter()
        .enumerate()
        .filter_map(|(i, shot)| {
            let (line, text, start, end) = span(timeline, lines, shot.line.as_deref()?)?;
            let words = shares(text, end - start);
            let from = words
                .iter()
                .position(|&(s, _)| start + s >= shot.start_ms)
                .unwrap_or(words.len());
            let to = words
                .iter()
                .rposition(|&(s, _)| start + s < shot.end_ms)
                .map_or(from, |k| (k + 1).max(from));
            Some(Ribbon {
                shot: i,
                line,
                words: from..to,
                past_ms: shot.end_ms.saturating_sub(end.max(shot.start_ms)),
            })
        })
        .collect()
}

/// When word `word` of script line `line` is said.
pub fn moment(timeline: &Timeline, lines: &[(String, String)], line: usize, word: usize) -> u64 {
    let Some((_, text, start, end)) = lines
        .get(line)
        .and_then(|(id, _)| span(timeline, lines, id))
    else {
        return 0;
    };
    start + shares(text, end - start).get(word).map_or(0, |&(s, _)| s)
}

/// A shot dropped on word `word` of script line `line`: it starts there.
pub fn dropped_on(shot: &ShotSpan, lines: &[(String, String)], line: usize, word: usize) -> Edit {
    let block = shot.block().to_string();
    let id = &lines[line].0;
    if shot.line.as_deref() == Some(id.as_str()) {
        Edit::Cue { block, word }
    } else {
        Edit::Move {
            block,
            after: id.clone(),
            word: Some(word),
        }
    }
}

/// A shot dropped in the pause after script line `line`: it plays after.
pub fn dropped_after(shot: &ShotSpan, lines: &[(String, String)], line: usize) -> Edit {
    let block = shot.block().to_string();
    let id = &lines[line].0;
    if shot.line.as_deref() == Some(id.as_str()) {
        Edit::Hold { block }
    } else {
        Edit::Move {
            block,
            after: id.clone(),
            word: None,
        }
    }
}

/// A shot's end dragged onto word `word` of its own line: as long as up
/// to that word's end. `None` for another line, or a shot that states no
/// length of its own.
pub fn stretched_to(
    timeline: &Timeline,
    lines: &[(String, String)],
    shot: &ShotSpan,
    line: usize,
    word: usize,
) -> Option<Edit> {
    let (_, text, start, end) = span(timeline, lines, shot.line.as_deref()?)?;
    if lines.get(line)?.0.as_str() != shot.line.as_deref()? || !shot.timed {
        return None;
    }
    let (_, until) = *shares(text, end - start).get(word)?;
    crate::timeline::stretched(shot, start + until)
}

/// The shot on screen at `ms`, by its index, and how far into it that is.
pub fn on_screen(timeline: &Timeline, ms: u64) -> Option<(usize, u64)> {
    timeline
        .shots
        .iter()
        .enumerate()
        .rev()
        .find(|(_, s)| (s.start_ms..s.end_ms).contains(&ms))
        .map(|(i, s)| (i, ms - s.start_ms))
}
