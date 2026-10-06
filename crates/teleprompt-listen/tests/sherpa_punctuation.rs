//! The punctuation model, run for real: set `TELEPROMPT_PUNCT_MODEL` to an
//! unpacked `sherpa-onnx-online-punct-en`. Skipped without it unless
//! `TELEPROMPT_REQUIRE_LISTEN` is set.
#![cfg(feature = "sherpa")]

use std::path::PathBuf;

#[test]
fn a_transcript_gets_capitals_and_punctuation() {
    let dir = std::env::var_os("TELEPROMPT_PUNCT_MODEL").map(PathBuf::from);
    assert!(
        dir.is_some() || std::env::var_os("TELEPROMPT_REQUIRE_LISTEN").is_none(),
        "TELEPROMPT_REQUIRE_LISTEN is set and TELEPROMPT_PUNCT_MODEL is not"
    );
    let Some(dir) = dir else { return };
    let out = teleprompt_listen::sherpa::punctuate(
        &dir,
        "WELCOME TO ACME LET ME SHOW YOU AROUND DEPLOYING IS ONE COMMAND",
    )
    .unwrap();
    assert!(out.starts_with("Welcome"), "{out}");
    assert!(out.contains(['.', ',', '?', '!']), "{out}");
    assert_eq!(out.split_whitespace().count(), 12, "{out}");
}
