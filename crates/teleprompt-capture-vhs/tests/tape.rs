//! A tape as things to do to a terminal.
//!
//! The parse is the compiler's, so what is tested here is only the other
//! half of the question: the compiler asks how long a line takes, and this
//! asks what it sends.

use teleprompt_capture_vhs::tape::{steps, Step, DEFAULT_TYPING_MS};

#[test]
fn typing_carries_its_text_and_the_speed_in_force() {
    let out = steps("Set TypingSpeed 20ms\nType \"hello\"\n");
    assert_eq!(
        out,
        vec![Step::Type {
            text: "hello".into(),
            per_char_ms: 20
        }]
    );
}

#[test]
fn a_tape_that_never_says_how_fast_it_types_types_at_the_default() {
    let out = steps("Type \"hi\"\n");
    assert_eq!(
        out,
        vec![Step::Type {
            text: "hi".into(),
            per_char_ms: DEFAULT_TYPING_MS
        }]
    );
}

/// `Type@100ms` and `Enter@50ms` carry their own speed, and it applies to
/// that command alone — the same reading the compiler's estimate uses.
#[test]
fn a_per_command_speed_applies_to_that_command_only() {
    let out = steps("Set TypingSpeed 20ms\nType@100ms \"a\"\nType \"b\"\n");
    let speeds: Vec<u64> = out
        .iter()
        .map(|s| match s {
            Step::Type { per_char_ms, .. } => *per_char_ms,
            _ => 0,
        })
        .collect();
    assert_eq!(speeds, vec![100, 20]);
}

#[test]
fn keys_become_the_bytes_a_terminal_sends() {
    let out = steps("Enter\nCtrl+C\nDown 3\nEscape\n");
    assert_eq!(
        out,
        vec![
            Step::Keys {
                bytes: b"\r".to_vec(),
                count: 1,
                per_key_ms: DEFAULT_TYPING_MS
            },
            Step::Keys {
                bytes: vec![0x03],
                count: 1,
                per_key_ms: DEFAULT_TYPING_MS
            },
            Step::Keys {
                bytes: b"\x1b[B".to_vec(),
                count: 3,
                per_key_ms: DEFAULT_TYPING_MS
            },
            Step::Keys {
                bytes: b"\x1b".to_vec(),
                count: 1,
                per_key_ms: DEFAULT_TYPING_MS
            },
        ]
    );
}

#[test]
fn a_sleep_is_a_wait_and_a_wait_is_a_deadline() {
    assert_eq!(steps("Sleep 1500ms\n"), vec![Step::Sleep(1_500)]);
    assert_eq!(
        steps("Set WaitTimeout 2s\nWait\n"),
        vec![Step::Quiet { timeout_ms: 2_000 }]
    );
    assert_eq!(
        steps("Wait@900ms\n"),
        vec![Step::Quiet { timeout_ms: 900 }],
        "`@` on a Wait is its timeout, not a typing speed"
    );
}

/// Settings the terminal applies and that cost no time contribute nothing
/// to send, and neither do comments — including the one that marks a span,
/// because spans are already split by the time a capture runs.
#[test]
fn what_costs_nothing_to_do_sends_nothing() {
    assert!(steps("Set FontSize 32\n# a comment\nHide\nShow\n").is_empty());
}
