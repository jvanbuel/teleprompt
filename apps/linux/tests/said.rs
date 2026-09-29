//! A line against what its take says, as the server sends it, for the
//! author to see what keeping it would change.

use teleprompt_gtk::api::Script;
use teleprompt_gtk::said::{markup, Change};

#[test]
fn a_line_said_otherwise_reads_with_its_changes() {
    let script: Script = serde_json::from_value(serde_json::json!({
        "lines": [{
            "id": "deploy", "text": "Deployment is one command.", "recorded": true,
            "said": "Deployment is just one command.",
            "said_diff": [
                { "kind": "same", "words": "Deployment is" },
                { "kind": "new", "words": "just" },
                { "kind": "same", "words": "one command." },
            ],
        }],
        "shots": [],
    }))
    .unwrap();
    assert_eq!(
        script.lines[0].said_diff,
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
    let changes = [
        Change::Same("A".into()),
        Change::Gone("<b>".into()),
        Change::New("d".into()),
        Change::Same("c.".into()),
    ];
    assert_eq!(
        markup(&changes),
        "A <span strikethrough=\"true\" foreground=\"#f28b82\">&lt;b&gt;</span> \
         <span weight=\"bold\" foreground=\"#81c995\">d</span> c."
    );
}
