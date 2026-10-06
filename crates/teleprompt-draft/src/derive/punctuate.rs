//! Punctuation from a punctuator's copy of the transcript, put back onto
//! the timed words.

use crate::derive::Word;

/// How far ahead a word is looked for in the punctuated text, past words
/// the punctuator added or rewrote.
const LOOKAHEAD: usize = 3;

/// `words`, each written as `punctuated` writes it: the same word, cased
/// and followed by punctuation. A word the punctuator changed otherwise is
/// kept as heard, so nothing said is lost and no time moves.
pub fn punctuate(words: &[Word], punctuated: &str) -> Vec<Word> {
    let tokens: Vec<&str> = punctuated.split_whitespace().collect();
    let mut next = 0;
    words
        .iter()
        .map(|word| {
            let key = bare(&word.text);
            let found = tokens
                .iter()
                .enumerate()
                .skip(next)
                .take(LOOKAHEAD + 1)
                .find(|(_, t)| bare(t) == key);
            let text = match found {
                Some((i, token)) => {
                    next = i + 1;
                    (*token).to_string()
                }
                None => {
                    next += 1;
                    as_heard(&word.text)
                }
            };
            Word {
                text,
                ..word.clone()
            }
        })
        .collect()
}

/// A word without case or punctuation, to compare two spellings of it.
fn bare(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A word kept as heard, lowered if the recognizer heard it in capitals,
/// since the words around it now have case.
fn as_heard(word: &str) -> String {
    let shouted = !word.chars().any(char::is_lowercase);
    if shouted && word != "I" && !word.starts_with("I'") {
        word.to_lowercase()
    } else {
        word.to_string()
    }
}
