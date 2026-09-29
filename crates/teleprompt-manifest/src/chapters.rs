//! The chapters of a video as a list to paste into a YouTube description.

use crate::NarrationManifest;

/// Fewest chapters YouTube shows.
const FEWEST: usize = 3;

/// Shortest chapter YouTube shows, in milliseconds.
const SHORTEST_MS: u64 = 10_000;

/// `0:00 Introduction` and so on, one chapter a line, and what would make
/// YouTube ignore the list: it wants three chapters or more, each at
/// least ten seconds long, the first at 0:00.
pub fn youtube(manifest: &NarrationManifest) -> (String, Vec<String>) {
    let chapters = &manifest.chapters;
    let mut text = String::new();
    let mut problems = Vec::new();
    if chapters.len() < FEWEST {
        problems.push(format!(
            "YouTube shows chapters only when there are at least three; this video has {}",
            chapters.len()
        ));
    }
    for (i, c) in chapters.iter().enumerate() {
        // The video opens on the first chapter, whatever its lead-in.
        let start = if i == 0 { 0 } else { c.start_ms.ms() };
        let end = chapters
            .get(i + 1)
            .map_or(manifest.duration_ms.ms(), |next| next.start_ms.ms());
        if end.saturating_sub(start) < SHORTEST_MS {
            problems.push(format!(
                "chapter `{}` is shorter than 10 seconds, which YouTube does not show",
                c.title
            ));
        }
        text.push_str(&format!("{} {}\n", clock(start), c.title));
    }
    (text, problems)
}

/// `m:ss`, or `h:mm:ss` from an hour, as YouTube writes a timestamp.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
