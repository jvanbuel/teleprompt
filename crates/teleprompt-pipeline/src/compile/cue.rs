//! Where in a narration a cued action starts (docs/design.md#cues).

use std::collections::BTreeMap;

use crate::schedule::NarrationInput;
use teleprompt_core::voice::spoken;
use teleprompt_core::{Diagnostic, PolicyKind};
use teleprompt_voice::WordTiming;

/// Where in a narration a cued action starts: from word timings when the
/// backend gave them ([`word_offset_ms`]), otherwise interpolated by
/// characters (docs/design.md#cues).
pub(super) fn at_offset_ms(
    phrase: &str,
    text: &str,
    narration: Option<&NarrationInput>,
    policy: PolicyKind,
    timed: Option<(&[WordTiming], &BTreeMap<String, String>)>,
) -> Result<Option<u64>, Diagnostic> {
    if policy != PolicyKind::Concurrent {
        return Err(Diagnostic::error(format!(
            "`cue=\"{phrase}\"` needs `policy=concurrent`, not `{policy}`"
        ))
        .with_help(
            "hold runs the action after the narration, and fit-action and trim-action \
             size it to fit; a shot only means something where the two run together",
        ));
    }
    let Some(narration) = narration else {
        return Err(
            Diagnostic::error(format!("`cue=\"{phrase}\"` has no narration to start in"))
                .with_help("a shot names a phrase in the paragraph above the block"),
        );
    };

    let Some(at) = text.find(phrase) else {
        return Err(Diagnostic::error(format!(
            "`cue=\"{phrase}\"` is not in the narration above it"
        ))
        .with_help(format!("the paragraph reads: {text}")));
    };

    if text.is_empty() {
        return Ok(None);
    }
    if let Some((words, pronounce)) = timed {
        if let Some(ms) =
            word_offset_ms(&spoken(phrase, pronounce), &spoken(text, pronounce), words)
        {
            return Ok(Some(ms));
        }
    }
    let fraction = text[..at].chars().count() as f64 / text.chars().count() as f64;
    Ok(Some(
        (narration.duration_ms as f64 * fraction).round() as u64
    ))
}

/// When the first word of `phrase` is said, from the backend's timings of
/// `text`, both as the voice was given them (pronunciations applied).
///
/// Words compare lowercased without punctuation. If the timed words match
/// the text's one for one, the word is taken by position; otherwise (a
/// backend that reads `0:12` as three words) it is the timed occurrence
/// nearest the same relative position. `None` when the word was not timed.
#[doc(hidden)]
pub fn word_offset_ms(phrase: &str, text: &str, words: &[WordTiming]) -> Option<u64> {
    fn norm(w: &str) -> String {
        w.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    let said: Vec<String> = text
        .split_whitespace()
        .map(norm)
        .filter(|w| !w.is_empty())
        .collect();
    let wanted: Vec<String> = phrase
        .split_whitespace()
        .map(norm)
        .filter(|w| !w.is_empty())
        .collect();
    let first = wanted.first()?;
    let at = (0..said.len()).find(|i| said[*i..].starts_with(&wanted))?;
    let timed: Vec<String> = words.iter().map(|w| norm(&w.word)).collect();

    if timed.len() == said.len() && timed[at] == *first {
        return Some(words[at].start_ms);
    }
    let relative = at as f64 / said.len().max(1) as f64;
    (0..timed.len())
        .filter(|j| timed[*j] == *first)
        .min_by(|a, b| {
            let d = |j: &usize| (*j as f64 / timed.len() as f64 - relative).abs();
            d(a).total_cmp(&d(b))
        })
        .map(|j| words[j].start_ms)
}
