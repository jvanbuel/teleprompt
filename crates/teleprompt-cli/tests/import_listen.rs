//! `import` hearing a real voice: the recognizer's fixture read twice, with
//! a command typed in the pause between. Needs a streaming zipformer model
//! in `TELEPROMPT_LISTEN_MODEL`; skipped without one unless
//! `TELEPROMPT_REQUIRE_LISTEN` is set.
#![cfg(feature = "listen")]

use std::path::PathBuf;

use teleprompt_draft::import::{run_import, Import, Words};
use teleprompt_voice::{wav, Pcm};

fn model() -> Option<PathBuf> {
    let dir = std::env::var_os("TELEPROMPT_LISTEN_MODEL").map(PathBuf::from);
    assert!(
        dir.is_some() || std::env::var_os("TELEPROMPT_REQUIRE_LISTEN").is_none(),
        "TELEPROMPT_REQUIRE_LISTEN is set and TELEPROMPT_LISTEN_MODEL is not"
    );
    dir
}

#[test]
fn a_spoken_session_becomes_two_lines_and_a_block_between() {
    let Some(model) = model() else {
        return;
    };
    let dir = teleprompt_testkit::test_dir("import-listen");
    teleprompt_cli::commands::new::scaffold(&dir).unwrap();

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../teleprompt-listen/tests/fixtures/two-lines.wav");
    let once = wav::decode(&std::fs::read(fixture).unwrap()).unwrap();
    let reading_ms = once.duration_ms();
    let gap = vec![0i16; once.sample_rate as usize * 4];
    let voice = dir.join("voice.wav");
    std::fs::write(
        &voice,
        wav::encode(&Pcm {
            samples: [once.samples.clone(), gap, once.samples.clone()].concat(),
            ..once.clone()
        }),
    )
    .unwrap();

    let mut cast = String::from("{\"version\": 2, \"width\": 80, \"height\": 24}\n");
    let mut t = (reading_ms + 1500) as f64 / 1000.0;
    for c in ["l", "s", "\\r"] {
        cast.push_str(&format!("[{t:.3}, \"i\", \"{c}\"]\n"));
        t += 0.08;
    }
    let cast_path = dir.join("session.cast");
    std::fs::write(&cast_path, cast).unwrap();

    let script = dir.join("scripts/session.md");
    let report = run_import(&Import {
        registry: teleprompt_cli::registry::registry(),
        reporter: &teleprompt_core::Silent,
        recording: &cast_path,
        with: None,
        voice: &voice,
        script: &script,
        words: Words::Model(&model),
        offset_ms: 0,
        punctuation: None,
        force: false,
    })
    .unwrap();
    let md = std::fs::read_to_string(&script).unwrap();
    assert_eq!(
        (report.lines, report.blocks, report.takes.len()),
        (2, 1, 2),
        "{md}"
    );
    let first = md.find("Welcome to acme").unwrap_or_else(|| panic!("{md}"));
    let block = md
        .find("include=recordings/session.cast#1")
        .unwrap_or_else(|| panic!("{md}"));
    let second = md.rfind("Welcome to acme").unwrap();
    assert!(first < block && block < second, "{md}");

    // With the punctuation model, the same session reads as sentences.
    let punct = std::env::var_os("TELEPROMPT_PUNCT_MODEL").map(PathBuf::from);
    assert!(
        punct.is_some() || std::env::var_os("TELEPROMPT_REQUIRE_LISTEN").is_none(),
        "TELEPROMPT_REQUIRE_LISTEN is set and TELEPROMPT_PUNCT_MODEL is not"
    );
    let Some(punct) = punct else { return };
    run_import(&Import {
        registry: teleprompt_cli::registry::registry(),
        reporter: &teleprompt_core::Silent,
        recording: &cast_path,
        with: None,
        voice: &voice,
        script: &script,
        words: Words::Model(&model),
        offset_ms: 0,
        punctuation: Some(&punct),
        force: true,
    })
    .unwrap();
    let md = std::fs::read_to_string(&script).unwrap();
    let line = md
        .lines()
        .find(|l| l.to_lowercase().starts_with("welcome to acme"))
        .unwrap_or_else(|| panic!("{md}"));
    let marks = line.matches(['.', ',', '?', '!']).count();
    assert!(marks >= 2, "punctuated within the line: {md}");
}
