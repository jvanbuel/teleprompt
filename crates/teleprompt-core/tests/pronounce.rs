//! Saying a word the way it is said rather than the way it is spelled.

use std::collections::BTreeMap;

use teleprompt_core::voice::spoken;

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// The case that prompted this: a synthetic voice reading `MWAA` as a word.
#[test]
fn an_acronym_is_replaced_with_how_it_is_said() {
    let said = spoken(
        "It works against MWAA, Composer and Astronomer.",
        &map(&[("MWAA", "em-double-you-ay-ay")]),
    );
    assert_eq!(
        said,
        "It works against em-double-you-ay-ay, Composer and Astronomer."
    );
}

/// Whole words only. A map entry is a word an author wants said
/// differently, not a substring: rewriting the middle of a longer word
/// produces something nobody wrote and nobody can hear the origin of.
#[test]
fn only_whole_words_are_replaced() {
    let m = map(&[("dub", "dubb")]);
    assert_eq!(spoken("dubious dubs, dub.", &m), "dubious dubs, dubb.");
}

/// Punctuation sits outside the word, so a replacement still fires next to
/// it — which is most of the places an acronym actually appears.
#[test]
fn punctuation_around_a_word_does_not_hide_it() {
    let m = map(&[("MWAA", "em-double-you-ay-ay")]);
    for (input, expect) in [
        ("(MWAA)", "(em-double-you-ay-ay)"),
        ("MWAA.", "em-double-you-ay-ay."),
        ("\"MWAA\"", "\"em-double-you-ay-ay\""),
        ("MWAA's", "em-double-you-ay-ay's"),
    ] {
        assert_eq!(spoken(input, &m), expect, "{input}");
    }
}

/// Case matters. `flowrs` at the start of a sentence and in the middle are
/// two different things to say, and an author who wants both writes both.
#[test]
fn matching_is_case_sensitive() {
    let m = map(&[("Flowrs", "flow-ers")]);
    assert_eq!(spoken("Flowrs and flowrs", &m), "flow-ers and flowrs");
}

/// An empty map is the common case and must not cost anything or change
/// anything.
#[test]
fn nothing_configured_changes_nothing() {
    let text = "Ordinary prose, unchanged.";
    assert_eq!(spoken(text, &BTreeMap::new()), text);
}
