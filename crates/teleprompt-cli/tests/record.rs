//! `record`'s terminal half: a program in a pseudo-terminal, recorded as
//! an asciicast with its keystrokes. (The microphone needs a microphone.)
#![cfg(unix)]

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use teleprompt_cli::cmd::record::record_pty;
use teleprompt_derive::{derive, read_cast, Options, Word};

/// Keys arriving as a person types them: one chunk at a time, with a pause
/// before each.
struct Typist(Vec<(u64, &'static str)>);

impl Read for Typist {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0.is_empty() {
            return Ok(0);
        }
        let (pause, keys) = self.0.remove(0);
        std::thread::sleep(Duration::from_millis(pause));
        buf[..keys.len()].copy_from_slice(keys.as_bytes());
        Ok(keys.len())
    }
}

#[test]
fn a_shell_session_is_recorded_with_its_keystrokes() {
    let typed: Vec<(u64, &'static str)> = [
        (300, "e"),
        (40, "c"),
        (40, "h"),
        (40, "o"),
        (40, " "),
        (40, "h"),
        (40, "i"),
        (40, "\r"),
        (500, "e"),
        (40, "x"),
        (40, "i"),
        (40, "t"),
        (40, "\r"),
    ]
    .to_vec();
    let shell = vec!["sh".to_string()];
    let (cast, _) = record_pty(
        &shell,
        Box::new(Typist(typed)),
        Box::new(std::io::sink()),
        (80, 24),
        &AtomicBool::new(false),
    )
    .unwrap();

    let trace = read_cast(&cast).unwrap_or_else(|e| panic!("{e}\n{cast}"));
    let typed: String = trace.input.iter().map(|(_, k)| k.as_str()).collect();
    assert_eq!(typed, "echo hi\rexit\r");
    assert!(cast.contains("hi\\r\\n"), "the output is recorded:\n{cast}");
    let times: Vec<u64> = trace.input.iter().map(|(t, _)| *t).collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
    assert!(times[8] - times[7] >= 450, "the pause is kept: {times:?}");

    // What derive makes of it: the command, and not the exit.
    let words = [Word {
        text: "HELLO".into(),
        start_ms: 0,
        end_ms: 200,
    }];
    let d = derive(&trace, &words, &Options::default());
    assert_eq!(d.beats[0].blocks.len(), 1);
    assert!(
        d.beats[0].blocks[0]
            .tape
            .contains("Type \"echo hi\"\nEnter\n"),
        "{:?}",
        d.beats
    );
    assert!(!d.beats[0].blocks[0].tape.contains("exit"));
}

/// Stopped from outside, as the app stops it: the shell ends and what was
/// recorded so far is returned.
#[test]
fn a_recording_stopped_from_outside_ends_its_shell() {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        flag.store(true, Ordering::SeqCst);
    });
    let shell = vec![
        "sh".to_string(),
        "-c".to_string(),
        "echo ready; sleep 30".to_string(),
    ];
    let began = Instant::now();
    let (cast, _) = record_pty(
        &shell,
        Box::new(Typist(Vec::new())),
        Box::new(std::io::sink()),
        (80, 24),
        &stop,
    )
    .unwrap();
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
    assert!(cast.contains("ready"), "{cast}");
}
