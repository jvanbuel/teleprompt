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
    assert!(steps("Set FontSize 32\n# a comment\n").is_empty());
}

/// The bug this file grew for. `Hide` is a gate on the recording: the
/// commands between it and `Show` run and are not watched running. For as
/// long as it parsed as "contributes nothing", `check` accepted it and the
/// capture dropped it — so a tape that hid its setup showed it. That is a
/// wrong picture, not a missing one.
#[test]
fn hiding_is_something_to_do_rather_than_nothing() {
    let out = steps("Hide\nType \"cd project\"\nEnter\nShow\nType \"ls\"\n");
    assert_eq!(out.first(), Some(&Step::Hide));
    assert!(
        out.contains(&Step::Show),
        "both ends of the gate reach the driver: {out:?}"
    );
    assert!(
        out.iter()
            .any(|s| matches!(s, Step::Type { text, .. } if text == "cd project")),
        "and what is hidden still runs: {out:?}"
    );
}

#[test]
fn a_requirement_and_a_clipboard_reach_the_driver() {
    let out = steps("Require flowrs\nCopy \"hello\"\nPaste\n");
    assert_eq!(
        out,
        [
            Step::Require("flowrs".into()),
            Step::Copy("hello".into()),
            Step::Paste,
        ]
    );
}

/// A tape that needs a tool this machine does not have is stopped before
/// the terminal opens. Recording it anyway produces a correctly-timed
/// video of `command not found`, which is worse than an error because it
/// looks like a video.
#[test]
fn a_missing_requirement_is_found_before_anything_runs() {
    let spans = [steps("Require definitely-not-a-real-program-9fa2\n")];
    assert_eq!(
        teleprompt_capture_vhs::tape::missing(&spans, "/usr/bin:/bin"),
        vec!["definitely-not-a-real-program-9fa2".to_string()]
    );
    assert!(
        teleprompt_capture_vhs::tape::missing(&[steps("Require sh\n")], "/usr/bin:/bin").is_empty()
    );
}
