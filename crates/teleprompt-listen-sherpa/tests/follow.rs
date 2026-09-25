//! The real recognizer, on a synthesized reading of two script lines. Needs
//! a streaming zipformer model: set `TELEPROMPT_LISTEN_MODEL` to its
//! directory. Skipped without one, unless `TELEPROMPT_REQUIRE_LISTEN` is set.
#![cfg(feature = "sherpa")]

use std::path::{Path, PathBuf};
use teleprompt_listen::{Follower, Position};
use teleprompt_listen_sherpa::SherpaRecognizer;

const LINES: &[&str] = &[
    "Welcome to Acme. Let me show you around.",
    "Deployment is one command, and it streams progress as it goes.",
];

fn model() -> Option<PathBuf> {
    let dir = std::env::var_os("TELEPROMPT_LISTEN_MODEL").map(PathBuf::from);
    assert!(
        dir.is_some() || std::env::var_os("TELEPROMPT_REQUIRE_LISTEN").is_none(),
        "TELEPROMPT_REQUIRE_LISTEN is set and TELEPROMPT_LISTEN_MODEL is not"
    );
    dir
}

/// Mono 16-bit PCM samples of a WAV file, as floats.
fn samples(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    let mut at = 12;
    while &bytes[at..at + 4] != b"data" {
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        at += 8 + len;
    }
    bytes[at + 8..]
        .chunks_exact(2)
        .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
        .collect()
}

#[test]
fn a_reading_of_two_lines_is_followed_to_the_end() {
    let Some(dir) = model() else {
        eprintln!("skipping: TELEPROMPT_LISTEN_MODEL is not set");
        return;
    };
    let recognizer = SherpaRecognizer::new(&dir).expect("the model loads");
    let mut follower = Follower::new(recognizer, LINES);

    let reading =
        samples(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/two-lines.wav"));
    let mut last = None;
    let mut early = None;
    // 100ms at a time, as a browser would send it, then a second of silence
    // for the recognizer to finish the last words.
    for (i, chunk) in reading
        .chunks(1600)
        .chain([[0.0f32; 16000].as_slice()])
        .enumerate()
    {
        if let Some(p) = follower.listen(chunk) {
            last = Some(p);
        }
        if i == 24 {
            early = last;
        }
    }
    // "Let me show" ends about 2.1 s in. A reader who gets no response
    // through their first sentence stops trusting the prompter, and some
    // small models drop the start of a stream entirely.
    let early = early.expect("the reader moved in the first 2.5 s");
    assert!(
        (early.line, early.word) >= (0, 6),
        "at 2.5 s the reader is only at {early:?}"
    );
    assert_eq!(last, Some(Position { line: 2, word: 0 }));
}
