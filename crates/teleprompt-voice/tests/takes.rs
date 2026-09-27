//! Recorded takes: one per line, kept beside the script as source, and
//! current only while the line still reads as it did when recorded.

use teleprompt_voice::takes::Takes;
use teleprompt_voice::Pcm;

fn pcm(ms: usize) -> Pcm {
    Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..ms * 48).map(|i| (i % 200) as i16).collect(),
    }
}

const TEXT: &str = "Welcome to Acme. Let me show you around.";

#[test]
fn a_saved_take_is_current_for_the_text_it_was_read_from() {
    let dir = teleprompt_testkit::test_dir("takes-saved");
    let takes = dir.join("takes");
    Takes::load(&takes)
        .unwrap()
        .save("welcome", TEXT, &pcm(1200))
        .unwrap();

    let loaded = Takes::load(&takes).unwrap();
    let take = loaded.current("welcome", TEXT).expect("current");
    assert_eq!(take.duration_ms, 1200);
    assert_eq!(take.text, TEXT);
}

/// Editing a line leaves its take behind: it no longer says what the line
/// says.
#[test]
fn a_take_of_other_words_is_not_current() {
    let dir = teleprompt_testkit::test_dir("takes-edited");
    let takes = dir.join("takes");
    Takes::load(&takes)
        .unwrap()
        .save("welcome", TEXT, &pcm(1200))
        .unwrap();
    let loaded = Takes::load(&takes).unwrap();
    assert!(loaded.current("welcome", "Welcome to Acme.").is_none());
    assert!(loaded.current("other", TEXT).is_none());
}

#[test]
fn no_takes_directory_is_no_takes() {
    let dir = teleprompt_testkit::test_dir("takes-none");
    let loaded = Takes::load(&dir.join("takes")).unwrap();
    assert!(loaded.current("welcome", TEXT).is_none());
}

/// Loading reads the sidecars alone, so `plan` never reads audio; the
/// audio is read, and checked against its hash, when it is used.
#[test]
fn audio_is_read_when_used_and_checked_against_its_record() {
    let dir = teleprompt_testkit::test_dir("takes-read");
    let takes = dir.join("takes");
    Takes::load(&takes)
        .unwrap()
        .save("welcome", TEXT, &pcm(500))
        .unwrap();
    let loaded = Takes::load(&takes).unwrap();
    let wav = loaded.read("welcome").unwrap();
    assert_eq!(&wav[..4], b"RIFF");

    std::fs::write(takes.join("welcome.wav"), b"RIFF but not the take").unwrap();
    let err = loaded.read("welcome").unwrap_err().to_string();
    assert!(
        err.contains("welcome.wav") && err.contains("changed"),
        "{err}"
    );

    std::fs::remove_file(takes.join("welcome.wav")).unwrap();
    let loaded = Takes::load(&takes).unwrap();
    assert!(loaded.current("welcome", TEXT).is_some());
    assert!(loaded.read("welcome").is_err());
}

#[test]
fn a_line_id_cannot_write_outside_the_takes_directory() {
    let dir = teleprompt_testkit::test_dir("takes-escape");
    let mut takes = Takes::load(&dir.join("takes")).unwrap();
    for id in ["../escape", "a/b", "", ".hidden"] {
        assert!(takes.save(id, TEXT, &pcm(10)).is_err(), "{id:?}");
    }
}

/// A line reworded since its take is stale: it has a take, of other words.
/// One never recorded has none, and is not.
#[test]
fn a_take_of_other_words_is_stale_and_no_take_is_not() {
    let dir = teleprompt_testkit::test_dir("takes-stale");
    let mut takes = Takes::load(&dir).unwrap();
    takes
        .save("welcome", "Welcome to Acme.", &pcm(500))
        .unwrap();
    assert!(takes.stale("welcome", "Welcome to Acme, everyone."));
    assert!(!takes.stale("welcome", "Welcome to Acme."));
    assert!(!takes.stale("deploy", "Deploying is one command."));
}
