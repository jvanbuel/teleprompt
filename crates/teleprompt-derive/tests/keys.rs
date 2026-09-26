use teleprompt_derive::{decode, Key};

fn chars(s: &str) -> Vec<Key> {
    s.chars().map(Key::Char).collect()
}

#[test]
fn printable_input_is_characters() {
    assert_eq!(decode("ls -la"), chars("ls -la"));
}

#[test]
fn control_bytes_are_named_keys() {
    assert_eq!(
        decode("\r\t\x7f\x08\x1b"),
        vec![
            Key::Named("Enter"),
            Key::Named("Tab"),
            Key::Named("Backspace"),
            Key::Named("Backspace"),
            Key::Named("Escape"),
        ]
    );
    assert_eq!(
        decode("\x03\x04\x12"),
        vec![Key::Ctrl('C'), Key::Ctrl('D'), Key::Ctrl('R')]
    );
}

/// Arrow keys arrive as escape sequences, in normal and in application
/// cursor mode, and each is one key.
#[test]
fn escape_sequences_are_one_key_each() {
    assert_eq!(
        decode("\x1b[A\x1bOB\x1b[C\x1b[D\x1b[3~\x1b[H\x1b[F\x1b[5~\x1b[6~"),
        ["Up", "Down", "Right", "Left", "Delete", "Home", "End", "PageUp", "PageDown"]
            .into_iter()
            .map(Key::Named)
            .collect::<Vec<_>>()
    );
    assert_eq!(decode("\x1bb"), vec![Key::Alt('b')]);
}

/// A paste arrives as one event holding many characters, and stays them.
#[test]
fn a_paste_is_its_characters() {
    assert_eq!(
        decode("echo hi\r"),
        [chars("echo hi"), vec![Key::Named("Enter")]].concat()
    );
}

/// Unrecognized escape sequences are dropped rather than typed as text.
#[test]
fn an_unknown_escape_sequence_is_dropped() {
    assert_eq!(decode("\x1b[200~x"), chars("x"));
}
