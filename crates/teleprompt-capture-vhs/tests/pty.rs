//! Driving a real terminal.
//!
//! The assertions are on what the terminal *wrote*, not on what the clip
//! looks like. That is the seam: the pty driver's job is to produce the
//! right terminal output and the renderer's is to draw it, and reading
//! text back out of drawn glyphs would need OCR to test a thing the
//! recording already states in plain bytes.

use std::path::PathBuf;

use teleprompt_capture::{sessions, Beat, Session};
use teleprompt_capture_vhs::{record, steps_of, terminal_for};
use teleprompt_core::Hash;

fn beat(span: &str, source: &str) -> Beat {
    let mut settings = std::collections::BTreeMap::new();
    settings.insert("columns".to_string(), "80".to_string());
    settings.insert("rows".to_string(), "20".to_string());
    settings.insert("settle_ms".to_string(), "600".to_string());
    Beat {
        span: span.into(),
        scene: "terminal".into(),
        adapter: "vhs".into(),
        session: None,
        key: Hash::of(span.as_bytes()),
        source: source.into(),
        duration_ms: 1_000,
        settings,
    }
}

fn shell_available() -> bool {
    let present = PathBuf::from("/bin/bash").exists() || PathBuf::from("/usr/bin/bash").exists();
    assert!(
        present || std::env::var_os("TELEPROMPT_REQUIRE_CAPTURE").is_none(),
        "TELEPROMPT_REQUIRE_CAPTURE is set and there is no bash to record"
    );
    present
}

/// Everything the terminal wrote during one span, as text.
fn wrote(recording: &teleprompt_capture_vhs::Recording, from_ms: u64, to_ms: u64) -> String {
    recording
        .events
        .iter()
        .filter(|(at, _)| *at >= from_ms && *at < to_ms)
        .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
        .collect()
}

fn run(session: &Session) -> teleprompt_capture_vhs::Recording {
    record(&terminal_for(session), &steps_of(session)).expect("the terminal opens")
}

/// What #18 is about: the tape runs, and what it did is on the screen.
#[test]
fn a_tape_that_types_something_makes_the_terminal_say_it() {
    if !shell_available() {
        eprintln!("skipping: no bash");
        return;
    }
    let beats = [beat(
        "a#0",
        "Set TypingSpeed 10ms\nType \"echo teleprompt-was-here\"\nEnter\nSleep 600ms\n",
    )];
    let plan = sessions(&beats, &|_| false);
    let recording = run(&plan[0]);

    let text = wrote(&recording, 0, u64::MAX);
    assert!(
        text.contains("teleprompt-was-here"),
        "nothing typed reached the terminal: {text:?}"
    );
    assert_eq!(recording.boundaries.len(), 1);
    assert!(recording.boundaries[0] > 0);
}

/// The reason a session is the unit rather than the beat. The second beat
/// runs in the terminal the first one left, so the program it started is
/// still running and the screen it drew is still there.
#[test]
fn a_session_is_one_terminal_for_all_of_its_beats() {
    if !shell_available() {
        eprintln!("skipping: no bash");
        return;
    }
    let beats = [
        beat(
            "a#0",
            "Set TypingSpeed 10ms\nType \"marker=first\"\nEnter\nSleep 400ms\n",
        ),
        // Reading back a variable the *previous* beat set is the whole
        // claim: a fresh shell would print an empty line.
        beat(
            "b#0",
            "Set TypingSpeed 10ms\nType \"echo saw-$marker\"\nEnter\nSleep 400ms\n",
        ),
    ];
    let plan = sessions(&beats, &|_| false);
    assert_eq!(plan.len(), 1, "one scene is one session");

    let recording = run(&plan[0]);
    assert_eq!(recording.boundaries.len(), 2);
    let second = wrote(&recording, recording.boundaries[0], recording.boundaries[1]);
    assert!(
        second.contains("saw-first"),
        "the second beat ran in a fresh shell, not the one the first left: \
         {second:?}"
    );
}

/// A cached step is replayed rather than skipped, so the step that *is*
/// wanted opens on the screen it was authored against.
#[test]
fn a_cached_step_is_still_run() {
    if !shell_available() {
        eprintln!("skipping: no bash");
        return;
    }
    let beats = [
        beat(
            "a#0",
            "Set TypingSpeed 10ms\nType \"marker=earlier\"\nEnter\nSleep 400ms\n",
        ),
        beat(
            "b#0",
            "Set TypingSpeed 10ms\nType \"echo saw-$marker\"\nEnter\nSleep 400ms\n",
        ),
    ];
    let already = beats[0].key;
    let plan = sessions(&beats, &|k| *k == already);

    assert_eq!(plan[0].steps.len(), 2, "both steps are in the session");
    assert!(!plan[0].steps[0].wanted);

    let recording = run(&plan[0]);
    let text = wrote(&recording, 0, u64::MAX);
    assert!(
        text.contains("saw-earlier"),
        "the cached step was skipped, so the kept one opened on the wrong \
         screen: {text:?}"
    );
}

/// A program that asks the terminal what it is gets an answer. Without
/// one it waits out its own timeout — seconds of a recording spent on a
/// question nobody was going to answer — and a TUI that turns on focus
/// reporting may never refresh at all.
#[test]
fn a_program_that_questions_the_terminal_is_answered() {
    if !shell_available() {
        eprintln!("skipping: no bash");
        return;
    }
    let beats = [beat(
        "a#0",
        // Ask for the background colour, then read whatever comes back.
        "Set TypingSpeed 5ms\n\
         Type \"printf '\\\\033]11;?\\\\033\\\\\\\\'; read -t 2 -r -d '\\\\\\\\' answer; \
         echo \\\"reply=${answer#*rgb:}\\\"\"\n\
         Enter\n\
         Sleep 1200ms\n",
    )];
    let plan = sessions(&beats, &|_| false);
    let text = wrote(&run(&plan[0]), 0, u64::MAX);

    assert!(
        text.contains("reply=0b0b"),
        "the terminal did not answer OSC 11, so a TUI asking it would sit \
         through its own timeout: {text:?}"
    );
}

/// A tape that runs the tool its script is documenting needs that tool on
/// `PATH`, and the shell a capture opens is not the one the author has in
/// front of them. `scene.<name>.env` is how a script says so.
#[test]
fn the_scene_can_put_things_in_the_environment() {
    if !shell_available() {
        eprintln!("skipping: no bash");
        return;
    }
    let mut b = beat(
        "a#0",
        "Set TypingSpeed 5ms\nType \"echo seen=$TELEPROMPT_TEST_MARKER\"\nEnter\nSleep 500ms\n",
    );
    b.settings
        .insert("env.TELEPROMPT_TEST_MARKER".to_string(), "yes".to_string());
    let plan = sessions(&[b], &|_| false);
    let text = wrote(&run(&plan[0]), 0, u64::MAX);

    assert!(
        text.contains("seen=yes"),
        "the scene's environment did not reach the shell: {text:?}"
    );
}
