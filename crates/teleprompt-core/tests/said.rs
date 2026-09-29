//! A line as its take says it: the script's words where the reader said
//! them, the recognizer's where they said something else, the script's
//! punctuation and capitals kept.

use teleprompt_core::said::{per_line, reworded};

#[test]
fn a_line_read_as_written_needs_no_rewording() {
    // The recognizer hears words, not capitals or punctuation.
    assert_eq!(
        reworded(
            "Welcome to Acme. Let me show you around.",
            "WELCOME TO ACME LET ME SHOW YOU AROUND"
        ),
        None
    );
    assert_eq!(reworded("It's here.", "it's here"), None);
}

#[test]
fn words_not_said_are_dropped_and_their_stop_kept() {
    assert_eq!(
        reworded(
            "Let me show you around the office!",
            "LET ME SHOW YOU AROUND"
        )
        .as_deref(),
        Some("Let me show you around!")
    );
}

#[test]
fn words_said_are_added_in_their_place() {
    assert_eq!(
        reworded(
            "Deployment is one command.",
            "DEPLOYMENT IS JUST ONE COMMAND"
        )
        .as_deref(),
        Some("Deployment is just one command.")
    );
}

#[test]
fn a_word_said_instead_replaces_it() {
    assert_eq!(
        reworded("Let me show you the way.", "LET ME SHOW YOU THE ROUTE").as_deref(),
        Some("Let me show you the route.")
    );
}

#[test]
fn a_word_said_first_in_a_sentence_is_capitalised() {
    assert_eq!(
        reworded("Welcome. Let me in.", "WELCOME SO LET ME IN").as_deref(),
        Some("Welcome. So let me in.")
    );
    assert_eq!(
        reworded("Let me in.", "NOW I LET ME IN").as_deref(),
        Some("Now I let me in.")
    );
}

#[test]
fn nothing_heard_rewords_nothing() {
    assert_eq!(reworded("Let me in.", ""), None);
    assert_eq!(reworded("Let me in.", "   "), None);
}

#[test]
fn a_dropped_sentence_end_replaces_the_stop_before_it() {
    assert_eq!(
        reworded("A line, and its end.", "A LINE").as_deref(),
        Some("A line.")
    );
    // A comma dropped is just dropped.
    assert_eq!(
        reworded("Go there, then here.", "GO THEN HERE").as_deref(),
        Some("Go then here.")
    );
}

/// A recognizer mishears: a word a letter or two off, or heard as two,
/// is the script's word, and no rewording.
#[test]
fn a_word_misheard_is_still_the_scripts() {
    assert_eq!(
        reworded(
            "Deployment is one command, and it streams progress as it goes.",
            "EMPLOYMENT IS ONE COMMAND AND ITS STREAMS PROGRESS AS IT GOES"
        ),
        None
    );
    assert_eq!(
        reworded(
            "Welcome to Acme. Let me show you around.",
            "WELCOME TO ACT ME LET ME SHOW YOU A ROUND"
        ),
        None
    );
    // But a short word is not another short word.
    assert!(reworded("It is here.", "IT IN HERE").is_some());
}

#[test]
fn a_word_heard_split_is_kept_whole_in_a_rewording() {
    assert_eq!(
        reworded(
            "Let me show you around the office!",
            "LET ME SHOW YOU A ROUND"
        )
        .as_deref(),
        Some("Let me show you around!")
    );
}

/// A take's transcript, across the lines it read: each word to the line
/// it was said in; one said between two lines, to the line after.
#[test]
fn a_takes_transcript_is_split_into_its_lines() {
    let lines = [
        "Welcome to Acme. Let me show you around the office!",
        "Deployment is one command.",
        "Then it's live.",
    ];
    assert_eq!(
        per_line(
            &lines,
            "WELCOME TO ACME LET ME SHOW YOU A ROUND SO EMPLOYMENT IS"
        ),
        [
            "welcome to acme let me show you a round",
            "so employment is",
            ""
        ]
    );
}

mod against_the_line {
    //! A line against what its take says, word by word, for the author to
    //! see what keeping it would change.

    use teleprompt_core::said::{diff, Change};

    #[test]
    fn words_kept_gone_and_new_in_reading_order() {
        assert_eq!(
            diff(
                "Let me show you the way there.",
                "Let me show you the route."
            ),
            [
                Change::Same("Let me show you the".into()),
                Change::Gone("way there.".into()),
                Change::New("route.".into()),
            ]
        );
    }

    #[test]
    fn a_word_added_between_is_new_there() {
        assert_eq!(
            diff(
                "Deployment is one command.",
                "Deployment is just one command."
            ),
            [
                Change::Same("Deployment is".into()),
                Change::New("just".into()),
                Change::Same("one command.".into()),
            ]
        );
    }

    #[test]
    fn the_same_words_are_one_run() {
        assert_eq!(
            diff("Deploy it.", "Deploy it."),
            [Change::Same("Deploy it.".into())]
        );
    }

    /// As the prompter's script sends it.
    #[test]
    fn each_run_is_sent_as_its_kind_and_words() {
        assert_eq!(
            serde_json::to_value(diff("Let me walk.", "Let me run.")).unwrap(),
            serde_json::json!([
                { "kind": "same", "words": "Let me" },
                { "kind": "gone", "words": "walk." },
                { "kind": "new", "words": "run." },
            ])
        );
    }
}
