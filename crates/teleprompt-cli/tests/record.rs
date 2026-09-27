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
        Arc::new(AtomicBool::new(false)),
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
        stop,
    )
    .unwrap();
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
    assert!(cast.contains("ready"), "{cast}");
}

/// The record key in the terminal: Ctrl+Shift+Space (like Ctrl+Space)
/// reaches the program as a NUL byte. It stops the recording, and the
/// shell never sees it.
#[test]
fn the_record_key_typed_in_the_terminal_stops_it() {
    let typed: Vec<(u64, &'static str)> = vec![
        (300, "e"),
        (40, "c"),
        (40, "h"),
        (40, "o"),
        (40, " "),
        (40, "h"),
        (40, "i"),
        (40, "\r"),
        (500, "\0"),
        // Never read: the recording is over.
        (20_000, "x"),
    ];
    let shell = vec!["sh".to_string()];
    let began = Instant::now();
    let (cast, _) = record_pty(
        &shell,
        Box::new(Typist(typed)),
        Box::new(std::io::sink()),
        (80, 24),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
    let trace = read_cast(&cast).unwrap();
    let typed: String = trace.input.iter().map(|(_, k)| k.as_str()).collect();
    assert_eq!(typed, "echo hi\r", "{cast}");
}

/// What the app reads, since it cannot wait on a terminal it did not
/// start itself: a recording that cannot start says so in its status file.
#[test]
fn a_recording_that_cannot_start_says_why_in_its_status() {
    let dir = teleprompt_testkit::test_dir("record-status");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let status = dir.join("status.json");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_teleprompt"))
        .current_dir(dir.path())
        .args([
            "record",
            "scripts/session.md",
            "--model",
            "no-such-model",
            "--status",
        ])
        .arg(&status)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&status).unwrap()).unwrap();
    assert_eq!(status["state"], "failed", "{status}");
    assert!(status["error"].as_str().unwrap().len() > 10, "{status}");
}
