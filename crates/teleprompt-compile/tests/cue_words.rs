//! `cue=` on timed words: when the phrase's first word is said.

use teleprompt_compile::word_offset_ms;
use teleprompt_voice::WordTiming;

fn timed(words: &[(&str, u64)]) -> Vec<WordTiming> {
    words
        .iter()
        .map(|(w, s)| WordTiming {
            word: w.to_string(),
            start_ms: *s,
            end_ms: s + 100,
        })
        .collect()
}

/// One for one: the phrase's word by position, case and punctuation aside.
#[test]
fn a_phrase_starts_when_its_first_word_is_said() {
    let words = timed(&[
        ("One", 0),
        ("command", 300),
        ("registers", 700),
        ("a", 1200),
        ("server", 1300),
    ]);
    assert_eq!(
        word_offset_ms(
            "registers a server",
            "One command registers a server.",
            &words
        ),
        Some(700)
    );
}

/// A word said twice is the occurrence the phrase is at.
#[test]
fn a_repeated_word_is_the_occurrence_the_phrase_names() {
    let words = timed(&[
        ("run", 0),
        ("it", 200),
        ("then", 400),
        ("run", 600),
        ("tests", 800),
    ]);
    assert_eq!(
        word_offset_ms("run tests", "run it, then run tests", &words),
        Some(600)
    );
}

/// Where the voice said more words than the text has — a number read out —
/// the nearest timed occurrence of the word is taken.
#[test]
fn a_backend_that_expands_words_is_matched_by_position() {
    let words = timed(&[
        ("Wait", 0),
        ("twelve", 300),
        ("hundred", 600),
        ("ms", 900),
        ("then", 1100),
        ("deploy", 1400),
    ]);
    assert_eq!(
        word_offset_ms("deploy", "Wait 1200 ms then deploy", &words),
        Some(1400)
    );
}

/// A word the backend did not time is left to interpolation.
#[test]
fn an_untimed_word_is_none() {
    let words = timed(&[("Hello", 0)]);
    assert_eq!(word_offset_ms("world", "Hello world", &words), None);
}
