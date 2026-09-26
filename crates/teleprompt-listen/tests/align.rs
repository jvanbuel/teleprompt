use teleprompt_listen::{Aligner, Position};

const SCRIPT: &[&str] = &[
    "Welcome to Acme. Let me show you around.",
    "Deployment is one command, and it streams progress as it goes.",
    "If a deploy goes wrong, rolling back takes one extra flag.",
];

#[test]
fn reading_the_opening_words_moves_past_them() {
    let mut a = Aligner::new(SCRIPT);
    assert_eq!(a.hear("welcome to acme"), Position { line: 0, word: 3 });
}

/// A recognizer finalizes an utterance and starts the next from silence; the
/// reader has not gone back to the top.
#[test]
fn a_committed_utterance_is_where_the_next_one_starts() {
    let mut a = Aligner::new(SCRIPT);
    a.hear("welcome to acme let me show you around");
    a.commit();
    assert_eq!(a.hear("deployment is"), Position { line: 1, word: 2 });
}

/// Recognizers mishear. A wrong word between right ones is the word the
/// script has there, not a reason to stop following.
#[test]
fn a_misheard_word_between_right_ones_does_not_stop_the_reader() {
    let mut a = Aligner::new(SCRIPT);
    assert_eq!(
        a.hear("welcome to acne let me"),
        Position { line: 0, word: 5 }
    );
}

/// Readers say things that are not in the script: a filler, a repeated
/// word. They do not push the reader on, and they do not lose them.
#[test]
fn a_word_not_in_the_script_is_passed_over() {
    let mut a = Aligner::new(SCRIPT);
    assert_eq!(
        a.hear("welcome to uh acme let me"),
        Position { line: 0, word: 5 }
    );
}

/// One word that happens to occur further on is not evidence the reader
/// jumped there; a prompter that lurches on every such word is unreadable.
#[test]
fn one_stray_word_from_further_on_does_not_jump() {
    let mut a = Aligner::new(SCRIPT);
    assert_eq!(
        a.hear("welcome to acme deployment"),
        Position { line: 0, word: 3 }
    );
}

/// A reader who drops a sentence and carries on further down is followed
/// there, once a run of words says so.
#[test]
fn a_skipped_sentence_is_followed_on_a_run_of_words() {
    let mut a = Aligner::new(SCRIPT);
    assert_eq!(
        a.hear("welcome to acme deployment is one command"),
        Position { line: 1, word: 4 }
    );
}

/// Words from before where the reader is are not a reason to scroll back:
/// a finished utterance is behind them for good.
#[test]
fn the_reader_is_never_moved_back_across_a_commit() {
    let mut a = Aligner::new(SCRIPT);
    a.hear("welcome to acme let me show you around deployment is");
    a.commit();
    assert_eq!(a.hear("welcome to acme"), Position { line: 1, word: 2 });
}

/// Scripts write numbers as digits; recognizers spell them out. Mid-sentence
/// a spelled number passes as a misheard word, but as the last word heard it
/// has to match, or the prompter lags behind the reader.
#[test]
fn digits_in_the_script_match_numbers_as_spoken() {
    let script = ["It takes 3 steps and 20 seconds, or 45 at most."];
    for (heard, word) in [
        ("it takes three", 3),
        ("it takes three steps and twenty", 6),
        ("it takes three steps and twenty seconds or forty five", 9),
    ] {
        let mut a = Aligner::new(&script);
        assert_eq!(a.hear(heard), Position { line: 0, word }, "{heard}");
    }
}

/// A hyphenated word is heard as its parts, and the reader is on it until
/// the last part is said. Positions count the line's words as written.
#[test]
fn a_hyphenated_word_is_heard_as_its_parts() {
    let script = ["The command-line tool is fast."];
    for (heard, word) in [
        ("the command", 1),
        ("the command line", 2),
        ("the command line tool", 3),
    ] {
        let mut a = Aligner::new(&script);
        assert_eq!(a.hear(heard), Position { line: 0, word }, "{heard}");
    }
}

/// Reading the last word puts the reader past the last line, which is how a
/// prompter knows the script is done.
#[test]
fn reading_to_the_end_is_past_the_last_line() {
    let mut a = Aligner::new(&["One two.", "Three four."]);
    a.hear("one two");
    a.commit();
    assert_eq!(a.hear("three four"), Position { line: 2, word: 0 });
}

/// Silence, or a hypothesis with nothing in it yet, leaves the reader put.
#[test]
fn an_empty_hypothesis_leaves_the_reader_where_they_are() {
    let mut a = Aligner::new(SCRIPT);
    a.hear("welcome to acme");
    a.commit();
    assert_eq!(a.hear(""), Position { line: 0, word: 3 });
}

/// A recognizer revises its partial hypothesis as it hears more; the latest
/// one is the truth, not the sum of them.
#[test]
fn a_revised_hypothesis_replaces_the_last() {
    let mut a = Aligner::new(SCRIPT);
    assert_eq!(a.hear("welcome to acme let"), Position { line: 0, word: 4 });
    assert_eq!(a.hear("welcome to acne"), Position { line: 0, word: 2 });
}

/// Only the words just ahead are candidates. A run that also occurs much
/// further down is a repeated phrase, not the reader skipping a page.
#[test]
fn a_run_far_ahead_of_the_reader_is_not_followed() {
    let filler = "and so on ".repeat(20);
    let script = ["Start here.", filler.as_str(), "The very same words."];
    let mut a = Aligner::new(&script);
    assert_eq!(
        a.hear("start the very same words"),
        Position { line: 0, word: 1 }
    );
}

/// A take can start at any line, to read it again: the reader is placed at
/// its first word, and nothing before it is in reach.
#[test]
fn a_take_can_start_at_a_later_line() {
    let mut a = Aligner::new(SCRIPT);
    a.hear("welcome to acme");
    a.restart_at(2);
    assert_eq!(a.hear("welcome to acme"), Position { line: 2, word: 0 });
    assert_eq!(a.hear("if a deploy"), Position { line: 2, word: 3 });
}
