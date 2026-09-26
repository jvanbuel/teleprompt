//! Cutting a take into its lines, from where the follower heard each line
//! begin and end. Words here are tone bursts, a comma is a short gap and a
//! paragraph break a long one.

use std::ops::Range;

use teleprompt_listen::{Position, TakeLog};

const RATE: u32 = 16_000;
/// How long after a word is said the follower reports it.
const LAG: usize = 6_400;

fn ms(ms: usize) -> usize {
    ms * RATE as usize / 1000
}

/// A reading: each line is words (ms) separated by `word_gap`, with a
/// `comma` pause before its last word; lines are `break_gap` apart.
/// Returns the audio and, per line, the span of its speech.
struct Reading {
    audio: Vec<f32>,
    spans: Vec<Range<usize>>,
    /// Per line, where its first and its last word end.
    first_end: Vec<usize>,
    last_end: Vec<usize>,
}

fn read(lines: &[usize], lead: usize, break_gap: usize) -> Reading {
    let mut audio = vec![0.0; ms(lead)];
    let (mut spans, mut first_end, mut last_end) = (vec![], vec![], vec![]);
    for (l, &words) in lines.iter().enumerate() {
        if l > 0 {
            audio.extend(vec![0.0; ms(break_gap)]);
        }
        let start = audio.len();
        for w in 0..words {
            if w > 0 {
                let gap = if w == words - 1 { 250 } else { 80 };
                audio.extend(vec![0.0; ms(gap)]);
            }
            let n = audio.len();
            audio.extend((0..ms(300)).map(|i| 0.3 * ((n + i) as f32 * 0.2).sin()));
            if w == 0 {
                first_end.push(audio.len());
            }
        }
        last_end.push(audio.len());
        spans.push(start..audio.len());
    }
    audio.extend(vec![0.0; ms(700)]);
    Reading {
        audio,
        spans,
        first_end,
        last_end,
    }
}

fn at(line: usize, word: usize) -> Position {
    Position { line, word }
}

/// The follower's reports for a reading of lines `from..`, each line's first
/// word and its end heard `LAG` late.
fn followed(r: &Reading, from: usize) -> TakeLog {
    let mut log = TakeLog::new(from);
    log.heard(at(from, 0), 0);
    for (i, (&first, &last)) in r.first_end.iter().zip(&r.last_end).enumerate() {
        log.heard(at(from + i, 1), first + LAG);
        log.heard(at(from + i + 1, 0), last + LAG);
    }
    log
}

/// Each cut keeps all of its line's speech, and not much else.
fn assert_holds(cut: &Range<usize>, speech: &Range<usize>) {
    assert!(
        cut.start <= speech.start && cut.end >= speech.end,
        "cut {cut:?} loses speech {speech:?}"
    );
    let slack = cut.len() - speech.len();
    assert!(
        slack <= ms(200),
        "cut {cut:?} carries {} ms of silence",
        slack * 1000 / RATE as usize
    );
}

#[test]
fn a_take_is_cut_into_its_lines_at_the_breaks() {
    let r = read(&[4, 3, 5], 500, 700);
    let lines = followed(&r, 0).lines(&r.audio, RATE);
    let numbers: Vec<usize> = lines.iter().map(|(l, _)| *l).collect();
    assert_eq!(numbers, [0, 1, 2]);
    for ((_, cut), speech) in lines.iter().zip(&r.spans) {
        assert_holds(cut, speech);
    }
}

/// The pause before a line's last word is closer to where the follower
/// reported the line's end than the break is; the break is the longer
/// silence, so the cut goes there and the last word stays with its line.
#[test]
fn a_pause_inside_a_line_is_not_taken_for_the_break() {
    let r = read(&[3, 3], 300, 450);
    let lines = followed(&r, 0).lines(&r.audio, RATE);
    assert_eq!(lines.len(), 2);
    assert_holds(&lines[0].1, &r.spans[0]);
    assert_holds(&lines[1].1, &r.spans[1]);
}

/// A take started part-way through the script numbers its lines from there.
#[test]
fn a_take_from_a_later_line_keeps_the_script_numbers() {
    let r = read(&[3, 4], 400, 600);
    let lines = followed(&r, 5).lines(&r.audio, RATE);
    let numbers: Vec<usize> = lines.iter().map(|(l, _)| *l).collect();
    assert_eq!(numbers, [5, 6]);
}

/// A line the reader stopped in the middle of is not a take of it.
#[test]
fn an_unfinished_line_is_not_kept() {
    let r = read(&[3, 4], 400, 600);
    let mut log = TakeLog::new(0);
    log.heard(at(0, 0), 0);
    log.heard(at(0, 1), r.first_end[0] + LAG);
    log.heard(at(1, 0), r.last_end[0] + LAG);
    log.heard(at(1, 2), r.first_end[1] + LAG);
    let lines = log.lines(&r.audio, RATE);
    let numbers: Vec<usize> = lines.iter().map(|(l, _)| *l).collect();
    assert_eq!(numbers, [0]);
}

/// A line the reader skipped: the follower jumps past it in one step, and
/// what was said there belongs to the next line, not to it.
#[test]
fn a_skipped_line_is_not_kept() {
    let r = read(&[3, 4], 400, 600);
    let mut log = TakeLog::new(0);
    log.heard(at(0, 0), 0);
    // Line 0 is skipped; the reader starts on line 1.
    log.heard(at(1, 2), r.first_end[0] + LAG);
    log.heard(at(2, 0), r.last_end[0] + LAG);
    let lines = log.lines(&r.audio, RATE);
    let numbers: Vec<usize> = lines.iter().map(|(l, _)| *l).collect();
    assert_eq!(numbers, [1]);
}

/// Nobody reads before a take starts, so its first line starts there: a
/// pause inside that line, before the follower heard it begin, is not a
/// break, however long.
#[test]
fn the_first_line_of_a_take_starts_with_the_take() {
    let tone = |ms_: usize| {
        (0..ms(ms_))
            .map(|i| 0.3 * (i as f32 * 0.2).sin())
            .collect::<Vec<_>>()
    };
    let quiet = |ms_: usize| vec![0.0; ms(ms_)];
    let mut audio = quiet(100);
    audio.extend(tone(800));
    audio.extend(quiet(400));
    audio.extend(tone(1000));
    let line_end = audio.len();
    audio.extend(quiet(700));
    let mut log = TakeLog::new(0);
    log.heard(at(0, 0), 0);
    // Heard begun only once the words after the pause were heard.
    log.heard(at(0, 4), ms(1700));
    log.heard(at(1, 0), line_end + LAG);
    let lines = log.lines(&audio, RATE);
    assert_eq!(lines.len(), 1);
    assert_holds(&lines[0].1, &(ms(100)..line_end));
}
