use teleprompt_derive::{derive, Mode, Options, Word};

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

#[test]
fn speech_alone_is_lines_cut_at_pauses() {
    let words = [say(0, "HELLO THERE"), say(3000, "THIS IS I")].concat();
    let d = derive(&[], &words, &Options::default());
    let lines: Vec<_> = d.lines().map(|l| l.text.as_str()).collect();
    assert_eq!(lines, ["Hello there.", "This is I."]);
    let first = d.lines().next().unwrap();
    assert_eq!((first.start_ms, first.end_ms), (0, 750));
    assert!(d.beats.iter().all(|b| b.blocks.is_empty()));
}

/// Mixed-case recognizer output is kept as it was heard.
#[test]
fn cased_speech_keeps_its_case() {
    let d = derive(&[], &say(0, "Deploy to AWS now"), &Options::default());
    assert_eq!(d.lines().next().unwrap().text, "Deploy to AWS now.");
}

/// A step taken in the pause after a line runs after it: `hold`.
#[test]
fn a_step_in_a_pause_holds_after_the_line_before_it() {
    let words = [say(0, "HELLO THERE"), say(4000, "DONE")].concat();
    let d = derive(&[2000], &words, &Options::default());

    assert_eq!(d.beats.len(), 2);
    let block = &d.beats[0].blocks[0];
    assert_eq!(block.mode, Mode::Hold);
    assert_eq!(block.cue, None);
    assert_eq!(block.steps, 0..1);
    assert!(d.beats[1].blocks.is_empty());
}

/// A step started while the line is being said runs with it, from the
/// words being said as it started.
#[test]
fn a_step_during_a_line_is_concurrent_and_cued_where_it_started() {
    // "now we list the files": "list" starts at 800 ms.
    let words = say(0, "NOW WE LIST THE FILES");
    let d = derive(&[850], &words, &Options::default());

    let block = &d.beats[0].blocks[0];
    assert_eq!(block.mode, Mode::Concurrent);
    assert_eq!(block.cue.as_deref(), Some("list the"));
    assert_eq!(d.lines().next().unwrap().text, "Now we list the files.");
}

/// A cue must find the phrase where the command started, so a phrase the
/// line says twice grows until it is unique.
#[test]
fn a_cue_grows_until_it_names_one_place() {
    // Words start every 400 ms: the second "the" is being said at 2050.
    let words = say(0, "LIST THE FILES THEN LIST THE FILES AGAIN");
    let d = derive(&[2050], &words, &Options::default());
    let text = &d.lines().next().unwrap().text;
    let cue = d.beats[0].blocks[0].cue.clone().unwrap();
    assert_eq!(cue, "the files again");
    assert_eq!(text.find(&cue), text.rfind(&cue));
}

/// Starting with the line needs no cue: `concurrent` starts together.
#[test]
fn a_step_at_the_start_of_a_line_needs_no_cue() {
    let d = derive(&[100], &say(0, "LOOK AT THIS"), &Options::default());
    let block = &d.beats[0].blocks[0];
    assert_eq!(block.mode, Mode::Concurrent);
    assert_eq!(block.cue, None);
}

#[test]
fn steps_before_any_speech_open_the_script_without_a_line() {
    let d = derive(&[0], &say(3000, "HELLO"), &Options::default());
    assert_eq!(d.beats.len(), 2);
    assert!(d.beats[0].line.is_none());
    assert_eq!(d.beats[0].blocks[0].mode, Mode::Hold);
}

/// Steps taken in the same pause are one block.
#[test]
fn steps_in_one_pause_are_one_block() {
    let words = [say(0, "FIRST"), say(8000, "LAST")].concat();
    let d = derive(&[2000, 3500, 9000], &words, &Options::default());
    assert_eq!(d.beats[0].blocks.len(), 1);
    assert_eq!(d.beats[0].blocks[0].steps, 0..2);
    assert_eq!(d.beats[1].blocks[0].steps, 2..3);
    assert_eq!(d.cuts(), [2]);
}

/// A step half during the line and one after it are two blocks: the
/// first runs with the line, the next after it.
#[test]
fn a_line_can_have_a_concurrent_and_a_held_block() {
    let words = [say(0, "ONE TWO"), say(6000, "THREE")].concat();
    let d = derive(&[100, 3000], &words, &Options::default());
    let modes: Vec<_> = d.beats[0].blocks.iter().map(|b| b.mode).collect();
    assert_eq!(modes, [Mode::Concurrent, Mode::Hold]);
    assert_eq!(d.cuts(), [1]);
}
