use teleprompt_derive::{derive, Block, Mode, Options, Trace, Word};

/// Words spoken from `at`, one every 400 ms, each 350 ms long.
fn say(at: u64, text: &str) -> Vec<Word> {
    text.split_whitespace()
        .enumerate()
        .map(|(i, w)| {
            let start = at + i as u64 * 400;
            Word {
                text: w.to_string(),
                start_ms: start,
                end_ms: start + 350,
            }
        })
        .collect()
}

/// `text` typed from `at`, one key every `gap` ms, then Enter.
fn typed(trace: &mut Trace, at: u64, gap: u64, text: &str) -> u64 {
    let mut t = at;
    for c in text.chars() {
        trace.input.push((t, c.to_string()));
        t += gap;
    }
    trace.input.push((t, "\r".to_string()));
    t
}

fn tapes(blocks: &[Block]) -> Vec<&str> {
    blocks.iter().map(|b| b.tape.as_str()).collect()
}

#[test]
fn speech_alone_is_lines_cut_at_pauses() {
    let words = [say(0, "HELLO THERE"), say(3000, "THIS IS I")].concat();
    let d = derive(&Trace::default(), &words, &Options::default());
    let lines: Vec<_> = d.lines().map(|l| l.text.as_str()).collect();
    assert_eq!(lines, ["Hello there.", "This is I."]);
    let first = d.lines().next().unwrap();
    assert_eq!((first.start_ms, first.end_ms), (0, 750));
    assert!(d.beats.iter().all(|b| b.blocks.is_empty()));
}

/// Mixed-case recognizer output is kept as it was heard.
#[test]
fn cased_speech_keeps_its_case() {
    let d = derive(
        &Trace::default(),
        &say(0, "Deploy to AWS now"),
        &Options::default(),
    );
    assert_eq!(d.lines().next().unwrap().text, "Deploy to AWS now.");
}

/// A command typed in the pause after a line runs after it: `hold`.
#[test]
fn a_command_in_a_pause_holds_after_the_line_before_it() {
    let mut trace = Trace::default();
    let enter = typed(&mut trace, 2000, 80, "ls");
    trace.output = vec![enter + 140, enter + 340];
    let words = [say(0, "HELLO THERE"), say(4000, "DONE")].concat();
    let d = derive(&trace, &words, &Options::default());

    assert_eq!(d.beats.len(), 2);
    let block = &d.beats[0].blocks[0];
    assert_eq!(block.mode, Mode::Hold);
    assert_eq!(block.cue, None);
    // Settles 300 ms after the last output, rounded up to 100 ms.
    assert_eq!(
        block.tape,
        "Set TypingSpeed 80ms\nType \"ls\"\nEnter\nSleep 700ms\n"
    );
    assert!(d.beats[1].blocks.is_empty());
}

/// A command started while the line is being said runs with it, from the
/// words being said as it started.
#[test]
fn a_command_during_a_line_is_concurrent_and_cued_where_it_started() {
    let mut trace = Trace::default();
    // "now we list the files": "list" starts at 800 ms.
    typed(&mut trace, 850, 50, "ls");
    let words = say(0, "NOW WE LIST THE FILES");
    let d = derive(&trace, &words, &Options::default());

    let block = &d.beats[0].blocks[0];
    assert_eq!(block.mode, Mode::Concurrent);
    assert_eq!(block.cue.as_deref(), Some("list the"));
    assert_eq!(d.lines().next().unwrap().text, "Now we list the files.");
}

/// A cue must find the phrase where the command started, so a phrase the
/// line says twice grows until it is unique.
#[test]
fn a_cue_grows_until_it_names_one_place() {
    let mut trace = Trace::default();
    typed(&mut trace, 2050, 50, "ls");
    // Words start every 400 ms: the second "the" is being said at 2050.
    let words = say(0, "LIST THE FILES THEN LIST THE FILES AGAIN");
    let d = derive(&trace, &words, &Options::default());
    let text = &d.lines().next().unwrap().text;
    let cue = d.beats[0].blocks[0].cue.clone().unwrap();
    assert_eq!(cue, "the files again");
    assert_eq!(text.find(&cue), text.rfind(&cue));
}

/// Starting with the line needs no cue: `concurrent` starts together.
#[test]
fn a_command_at_the_start_of_a_line_needs_no_cue() {
    let mut trace = Trace::default();
    typed(&mut trace, 100, 50, "ls");
    let d = derive(&trace, &say(0, "LOOK AT THIS"), &Options::default());
    let block = &d.beats[0].blocks[0];
    assert_eq!(block.mode, Mode::Concurrent);
    assert_eq!(block.cue, None);
}

#[test]
fn commands_before_any_speech_open_the_script_without_a_line() {
    let mut trace = Trace::default();
    typed(&mut trace, 0, 50, "clear");
    let d = derive(&trace, &say(3000, "HELLO"), &Options::default());
    assert_eq!(d.beats.len(), 2);
    assert!(d.beats[0].line.is_none());
    assert_eq!(d.beats[0].blocks[0].mode, Mode::Hold);
}

/// Commands typed in the same pause are one tape, each waiting as long as
/// the recording did before the next.
#[test]
fn commands_in_one_pause_share_a_tape() {
    let mut trace = Trace::default();
    let e1 = typed(&mut trace, 2000, 50, "ls");
    typed(&mut trace, e1 + 1500, 50, "pwd");
    let words = [say(0, "FIRST"), say(8000, "LAST")].concat();
    let d = derive(&trace, &words, &Options::default());
    assert_eq!(d.beats[0].blocks.len(), 1);
    assert_eq!(
        d.beats[0].blocks[0].tape,
        "Set TypingSpeed 50ms\nType \"ls\"\nEnter\nSleep 1500ms\nType \"pwd\"\nEnter\nSleep 300ms\n"
    );
}

/// A command half during the line and half after it is two tapes: the
/// first runs with the line, the next after it.
#[test]
fn a_line_can_have_a_concurrent_and_a_held_tape() {
    let mut trace = Trace::default();
    typed(&mut trace, 100, 50, "ls");
    typed(&mut trace, 3000, 50, "pwd");
    let words = [say(0, "ONE TWO"), say(6000, "THREE")].concat();
    let d = derive(&trace, &words, &Options::default());
    let modes: Vec<_> = d.beats[0].blocks.iter().map(|b| b.mode).collect();
    assert_eq!(modes, [Mode::Concurrent, Mode::Hold]);
}

/// Typing mistakes fixed with Backspace are the corrected text; a long
/// pause while typing is kept as a `Sleep`.
#[test]
fn backspaces_edit_the_text_and_pauses_are_kept() {
    let mut trace = Trace::default();
    for (t, k) in [
        (2000, "l"),
        (2050, "x"),
        (2100, "\x7f"),
        (2150, "s"),
        (3500, " "),
        (3550, "-"),
        (3600, "l"),
        (3650, "\r"),
    ] {
        trace.input.push((t, k.to_string()));
    }
    let d = derive(&trace, &say(0, "HI"), &Options::default());
    assert_eq!(
        tapes(&d.beats[0].blocks),
        ["Set TypingSpeed 50ms\nType \"ls\"\nSleep 1300ms\nType \" -l\"\nEnter\nSleep 300ms\n"]
    );
}

/// Named keys stay keys, repeats are counted, and the `exit` that ended
/// the recording is not part of the script.
#[test]
fn keys_are_kept_and_the_closing_exit_is_dropped() {
    let mut trace = Trace::default();
    for (t, k) in [
        (2000, "\x1b[A"),
        (2100, "\x1b[A"),
        (2200, "\t"),
        (2300, "\r"),
    ] {
        trace.input.push((t, k.to_string()));
    }
    typed(&mut trace, 9000, 50, "exit");
    let d = derive(&trace, &say(0, "HI"), &Options::default());
    assert_eq!(
        tapes(&d.beats[0].blocks),
        ["Set TypingSpeed 50ms\nUp 2\nTab\nEnter\nSleep 300ms\n"]
    );
    assert_eq!(d.beats.len(), 1);
}

/// Quotes in a command pick a delimiter the command does not use, and a
/// backslash survives VHS's escapes.
#[test]
fn typed_text_is_quoted_so_vhs_reads_it_back() {
    let mut trace = Trace::default();
    typed(&mut trace, 2000, 50, r#"echo "it's" \n"#);
    let d = derive(&trace, &say(0, "HI"), &Options::default());
    let tape = &d.beats[0].blocks[0].tape;
    let line = tape.lines().nth(1).unwrap();
    assert_eq!(line, r#"Type `echo "it's" \\n`"#);
}
