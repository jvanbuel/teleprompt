//! A script read by its voice: where the reading is at a point in a line's
//! audio, which shots that reaches, and how far the voice has got making
//! the lines. The window plays the audio; this says what it means.

use std::collections::BTreeSet;

use crate::api::{Line, Position, Script, Source};

/// The word being said `ms` into a line whose words start at `starts`:
/// the last one begun.
pub fn word_at(starts: &[u64], ms: u64) -> usize {
    starts
        .iter()
        .take_while(|&&start| start <= ms)
        .count()
        .saturating_sub(1)
}

/// The shots not yet `started` whose cue lies between `from`, where the
/// reading began, and `at`, where it is now: as a reader's voice would
/// start them.
pub fn due(
    script: &Script,
    from: Position,
    at: Position,
    started: &BTreeSet<String>,
) -> Vec<String> {
    script
        .shots
        .iter()
        .filter(|s| (from..=at).contains(&s.at) && !started.contains(&s.shot))
        .map(|s| s.shot.clone())
        .collect()
}

/// How a line's audio stands, for its mark in the gutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Read from its take.
    Take,
    /// Made by the voice.
    Voiced,
    /// The voice has yet to make it.
    Unvoiced,
}

/// `None` from a server that says nothing of voices.
pub fn mark(line: &Line) -> Option<Mark> {
    let audio = line.audio.as_ref()?;
    Some(match (audio.source, audio.ready) {
        (Source::Take, _) => Mark::Take,
        (Source::Voice, true) => Mark::Voiced,
        (Source::Voice, false) => Mark::Unvoiced,
    })
}

/// How many lines have their audio, of how many that have any.
pub fn made(script: &Script) -> (usize, usize) {
    let audio: Vec<_> = script
        .lines
        .iter()
        .filter_map(|l| l.audio.as_ref())
        .collect();
    (audio.iter().filter(|a| a.ready).count(), audio.len())
}

/// The lines whose audio the voice has yet to make, in order.
pub fn unmade(script: &Script) -> Vec<usize> {
    script
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.audio.as_ref().is_some_and(|a| !a.ready))
        .map(|(i, _)| i)
        .collect()
}

/// The colours a speaker's mark takes, one per name: none is the cue's
/// amber, the tally's red, the recorded green or the heard blue, which
/// mean other things on the glass. `apps/DESIGN.md` has the same list.
pub const SPEAKER_COLOURS: [&str; 4] = ["#c58af9", "#4dd0c8", "#f28bd0", "#d7b98e"];

/// Speaker `name`'s colour: the same in every app, since it is chosen by
/// the name's letters rather than by order of appearance.
pub fn speaker_colour(name: &str) -> &'static str {
    let sum: u32 = name.bytes().map(u32::from).sum();
    SPEAKER_COLOURS[sum as usize % SPEAKER_COLOURS.len()]
}

/// The letter a speaker's mark shows.
pub fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default()
}

/// A length as minutes and seconds: "1:23".
pub fn clock(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
