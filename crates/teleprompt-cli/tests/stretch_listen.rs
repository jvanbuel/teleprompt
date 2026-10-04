//! A voice stretched for `fit-line` still says its words: the speech model
//! hears them, faster and slower, as in the original. Needs the
//! model, as the other `_listen` tests do.
#![cfg(feature = "listen")]

use std::path::PathBuf;

use teleprompt_voice::stretch::stretch;

fn model() -> Option<PathBuf> {
    let dir = std::env::var_os("TELEPROMPT_LISTEN_MODEL").map(PathBuf::from);
    assert!(
        dir.is_some() || std::env::var_os("TELEPROMPT_REQUIRE_LISTEN").is_none(),
        "TELEPROMPT_REQUIRE_LISTEN is set and TELEPROMPT_LISTEN_MODEL is not"
    );
    dir
}

fn heard(dir: &std::path::Path, pcm: &teleprompt_plugin::voice::Pcm) -> String {
    let samples: Vec<f32> = pcm
        .samples
        .iter()
        .map(|&s| f32::from(s) / 32768.0)
        .collect();
    let words = teleprompt_listen_sherpa::transcribe(dir, &samples).unwrap();
    words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn a_stretched_reading_says_the_same_words() {
    let Some(dir) = model() else {
        return;
    };
    let wav = std::fs::read("../teleprompt-listen-sherpa/tests/fixtures/two-lines.wav").unwrap();
    let reading = teleprompt_plugin::voice::wav::decode(&wav).unwrap();
    let original = heard(&dir, &reading);
    assert!(original.starts_with("WELCOME TO ACME"), "{original}");
    // As `keep what you said` hears a take: a word a letter off is the same
    // word, since a small streaming model mishears as much as that at a join
    // between two lines even in the original.
    for tempo in [900u32, 1080, 1150] {
        let words = heard(&dir, &stretch(&reading, tempo));
        assert_eq!(
            teleprompt_core::said::reworded(&original, &words),
            None,
            "at {tempo}: {words}"
        );
    }
}
