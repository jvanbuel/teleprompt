//! Transcribing a whole recording with word timings. Needs a model, as
//! `follow.rs` does.
#![cfg(feature = "sherpa")]

use std::path::{Path, PathBuf};
use teleprompt_listen::sherpa::transcribe;

fn model() -> Option<PathBuf> {
    let dir = std::env::var_os("TELEPROMPT_LISTEN_MODEL").map(PathBuf::from);
    assert!(
        dir.is_some() || std::env::var_os("TELEPROMPT_REQUIRE_LISTEN").is_none(),
        "TELEPROMPT_REQUIRE_LISTEN is set and TELEPROMPT_LISTEN_MODEL is not"
    );
    dir
}

fn samples(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    let mut at = 12;
    while &bytes[at..at + 4] != b"data" {
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        at += 8 + len;
    }
    bytes[at + 8..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
        .collect()
}

/// The fixture read twice with three seconds between: every word, in
/// order, each timed from the start of the recording, across the endpoint
/// the pause makes.
#[test]
fn a_recording_becomes_timed_words() {
    let Some(dir) = model() else {
        return;
    };
    let once = samples(Path::new("tests/fixtures/two-lines.wav"));
    let gap = 3 * 16_000;
    let audio = [once.clone(), vec![0.0; gap], once.clone()].concat();
    let words = transcribe(&dir, &audio).unwrap();

    let text: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    let reading = "WELCOME TO ACME LET ME SHOW YOU";
    assert_eq!(text[..7].join(" "), reading, "{text:?}");
    let second = text
        .iter()
        .rposition(|w| *w == "WELCOME")
        .filter(|&i| i > 0)
        .unwrap_or_else(|| panic!("{text:?}"));
    assert_eq!(text[second..second + 7].join(" "), reading);
    assert!(
        text.ends_with(&["GOES"]),
        "the last word is heard out: {text:?}"
    );

    // "WELCOME" starts about 0.36 s in, and again one reading and the gap
    // later.
    let first = words[0].start_ms;
    assert!((300..450).contains(&first), "{first}");
    let length = once.len() as u64 * 1000 / 16_000 + 3000;
    let again = words[second].start_ms;
    assert!(
        again.abs_diff(first + length) < 150,
        "{again} vs {}",
        first + length
    );

    for pair in words.windows(2) {
        assert!(pair[0].start_ms < pair[0].end_ms, "{pair:?}");
        assert!(pair[0].end_ms <= pair[1].start_ms, "{pair:?}");
    }
}
