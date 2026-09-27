//! A line against what its take says, word by word, for the author to see
//! what keeping it would change.

use teleprompt_gtk::said::{diff, Change};

#[test]
fn words_kept_gone_and_new_in_reading_order() {
    let d = diff(
        "Let me show you the way there.",
        "Let me show you the route.",
    );
    assert_eq!(
        d,
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

/// As Pango markup: what goes struck through, what comes in bold, and
/// the text escaped.
#[test]
fn a_diff_is_marked_up_for_the_dialog() {
    let markup = teleprompt_gtk::said::markup(&diff("A <b> c.", "A d c."));
    assert_eq!(
        markup,
        "A <span strikethrough=\"true\" foreground=\"#f28b82\">&lt;b&gt;</span> \
         <span weight=\"bold\" foreground=\"#81c995\">d</span> c."
    );
}
