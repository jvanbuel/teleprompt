//! A video over its `timing.length_ms`: by how much, what pictures hold,
//! and about how much narration to cut (docs/design.md#led-by-the-picture).

use teleprompt_core::program::ChapterInfo;
use teleprompt_core::PolicyKind;
use teleprompt_schedule::Timeline;

use crate::NarrationDetail;

fn seconds(ms: u64) -> String {
    format!("{:.1} s", ms as f64 / 1000.0)
}

/// The warning for `timeline` running past `length_ms`, or `None` within
/// it. Time a `fit-line` picture holds cannot be cut by rewording, so the
/// words to cut are counted against the rest of the narration.
pub fn over_length(
    timeline: &Timeline,
    narration: &[NarrationDetail],
    chapters: &[ChapterInfo],
    length_ms: u64,
) -> Option<String> {
    let total = timeline.duration_ms.ms();
    let over = total.checked_sub(length_ms).filter(|&o| o > 0)?;
    let detail = |line: &str| narration.iter().find(|d| d.line_id == line);

    let (mut held_ms, mut held) = (0u64, Vec::new());
    let (mut words, mut spoken_ms) = (0usize, 0u64);
    let mut by_chapter: Vec<(usize, u64)> = Vec::new();
    let mut chapter = 0usize;
    for e in &timeline.entries {
        let line = e.narration.as_ref();
        if let Some(d) = line.and_then(|n| detail(&n.line)) {
            chapter = d.chapter_index;
        }
        match by_chapter.last_mut() {
            Some((c, ms)) if *c == chapter => *ms += e.duration_ms.ms(),
            _ => by_chapter.push((chapter, e.duration_ms.ms())),
        }
        if e.policy == PolicyKind::FitLine {
            held_ms += e.duration_ms.ms();
            held.push(format!("`{}`", e.item));
        } else if let Some(n) = line {
            spoken_ms += n.duration_ms.ms();
            words += detail(&n.line).map_or(0, |d| d.text.split_whitespace().count());
        }
    }

    let mut out = format!(
        "the video runs {} against its {} length, over by {}",
        seconds(total),
        seconds(length_ms),
        seconds(over)
    );
    if held_ms > 0 {
        out.push_str(&format!(
            ": {} is held by pictures ({}), and the rest is narration",
            seconds(held_ms),
            held.join(", ")
        ));
    }
    if words > 0 && spoken_ms > 0 {
        let cut = ((over as f64 * words as f64 / spoken_ms as f64).ceil() as usize).min(words);
        out.push_str(&format!("; cut about {cut} of its {words} words"));
    }
    let parts: Vec<String> = by_chapter
        .iter()
        .filter_map(|(c, ms)| {
            chapters
                .get(*c)
                .map(|ch| format!("{} {}", ch.title, seconds(*ms)))
        })
        .collect();
    if !parts.is_empty() {
        out.push_str(&format!(". By chapter: {}", parts.join(", ")));
    }
    Some(out)
}
