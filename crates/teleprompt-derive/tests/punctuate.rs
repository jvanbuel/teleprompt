use teleprompt_derive::{derive, punctuate, Options, Word};

fn heard(text: &str) -> Vec<Word> {
    text.split_whitespace()
        .enumerate()
        .map(|(i, w)| Word {
            text: w.to_string(),
            start_ms: i as u64 * 400,
            end_ms: i as u64 * 400 + 350,
        })
        .collect()
}

fn texts(words: &[Word]) -> Vec<&str> {
    words.iter().map(|w| w.text.as_str()).collect()
}

/// Each word takes the punctuated form of itself, and keeps its time.
#[test]
fn punctuation_lands_on_the_words_it_follows() {
    let words = heard("WELCOME TO ACME LET ME SHOW YOU AROUND");
    let out = punctuate(&words, "Welcome to Acme. Let me show you around.");
    assert_eq!(
        texts(&out),
        ["Welcome", "to", "Acme.", "Let", "me", "show", "you", "around."]
    );
    assert!(out
        .iter()
        .zip(&words)
        .all(|(a, b)| (a.start_ms, a.end_ms) == (b.start_ms, b.end_ms)));
}

/// A word the punctuator changed beyond case and punctuation is kept as
/// heard, and the rest still line up.
#[test]
fn a_word_the_punctuator_rewrote_is_kept_as_heard() {
    let words = heard("i said its done ok");
    let out = punctuate(&words, "I said, it's done, okay?");
    assert_eq!(texts(&out), ["I", "said,", "it's", "done,", "ok"]);
}

/// Nothing is lost when the punctuator returns fewer or more words.
#[test]
fn a_mismatched_count_keeps_every_word() {
    let words = heard("one two three");
    assert_eq!(
        texts(&punctuate(&words, "One, two.")),
        ["One,", "two.", "three"]
    );
    assert_eq!(texts(&punctuate(&words, "")), ["one", "two", "three"]);
}

/// A line cut at a pause after a comma ends in a full stop, not in both.
#[test]
fn a_line_ending_in_a_comma_ends_in_a_full_stop() {
    let words = punctuate(&heard("first we build then"), "First, we build, then");
    let mut words = words;
    // A pause after "build,".
    for w in &mut words[3..] {
        w.start_ms += 3000;
        w.end_ms += 3000;
    }
    let d = derive(&[], &words, &Options::default());
    let lines: Vec<&str> = d
        .beats
        .iter()
        .filter_map(|b| b.line.as_ref())
        .map(|l| l.text.as_str())
        .collect();
    assert_eq!(lines, ["First, we build.", "Then."]);
}
