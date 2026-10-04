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

/// What the recognizer heard is kept with the take: where it is other
/// words than the line's, the line as it was said.
#[test]
fn a_take_heard_saying_other_words_offers_them() {
    let dir = teleprompt_testkit::test_dir("takes-heard");
    let takes = dir.join("takes");
    let text = "Let me show you around the office!";
    let mut store = Takes::load(&takes).unwrap();
    store
        .save_heard("welcome", text, "LET ME SHOW YOU AROUND", &pcm(500))
        .unwrap();
    store
        .save_heard("deploy", "Deploy it.", "DEPLOY IT", &pcm(500))
        .unwrap();
    store.save("plain", "No ears.", &pcm(500)).unwrap();

    let loaded = Takes::load(&takes).unwrap();
    assert_eq!(
        loaded.said("welcome", text).as_deref(),
        Some("Let me show you around!")
    );
    assert_eq!(loaded.said("deploy", "Deploy it."), None, "said as written");
    assert_eq!(loaded.said("plain", "No ears."), None, "nothing heard");
    assert_eq!(loaded.said("welcome", "Reworded since."), None, "stale");
    // A take saved without hearing it says nothing of it.
    let sidecar = std::fs::read_to_string(takes.join("plain.json")).unwrap();
    assert!(!sidecar.contains("heard"), "{sidecar}");
}

/// The line reworded to what was said: the take is current for it, the
/// same audio.
#[test]
fn a_take_retexted_is_current_for_its_new_words() {
    let dir = teleprompt_testkit::test_dir("takes-retext");
    let takes = dir.join("takes");
    let text = "Let me show you around the office!";
    let mut store = Takes::load(&takes).unwrap();
    store
        .save_heard("welcome", text, "LET ME SHOW YOU AROUND", &pcm(500))
        .unwrap();
    let said = store.said("welcome", text).unwrap();
    store.retext("welcome", &said).unwrap();

    let loaded = Takes::load(&takes).unwrap();
    assert!(loaded.current("welcome", &said).is_some());
    assert!(loaded.current("welcome", text).is_none());
    assert_eq!(loaded.said("welcome", &said), None);
    assert!(loaded.read("welcome").is_ok(), "the audio is still its own");
    assert!(Takes::load(&takes).unwrap().retext("nope", "x").is_err());
}

#[test]
fn takes_are_listed_by_line() {
    let dir = teleprompt_testkit::test_dir("takes-list");
    let mut takes = teleprompt_voice::takes::Takes::load(dir.path()).unwrap();
    let pcm = teleprompt_voice::Pcm {
        sample_rate: 16_000,
        channels: 1,
        samples: vec![0; 16_000],
    };
    takes.save("b", "Bee.", &pcm).unwrap();
    takes.save("a", "Ay.", &pcm).unwrap();
    let listed: Vec<(&str, &str)> = takes.iter().map(|(id, t)| (id, t.text.as_str())).collect();
    assert_eq!(listed, [("a", "Ay."), ("b", "Bee.")]);
}

/// Keeping a take puts the one it replaced aside, and `restore` puts it
/// back: a botched take never costs a good one.
#[test]
fn a_take_that_replaced_another_can_be_undone() {
    let dir = teleprompt_testkit::test_dir("takes-undo");
    let mut takes = Takes::load(&dir).unwrap();
    takes.save("welcome", "Welcome.", &pcm(100)).unwrap();
    let first = takes.read("welcome").unwrap();
    takes.save("welcome", "Welcome.", &pcm(300)).unwrap();
    assert_ne!(takes.read("welcome").unwrap(), first);

    assert!(takes.restore("welcome").unwrap());
    assert_eq!(takes.read("welcome").unwrap(), first);
    assert_eq!(Takes::load(&dir).unwrap().read("welcome").unwrap(), first);
    // Once: what was put back is the take now.
    assert!(!takes.restore("welcome").unwrap());
    assert_eq!(takes.read("welcome").unwrap(), first);
}

/// A line's first take, undone, leaves the line with none.
#[test]
fn a_first_take_undone_leaves_no_take() {
    let dir = teleprompt_testkit::test_dir("takes-undo-first");
    let mut takes = Takes::load(&dir).unwrap();
    takes.save("welcome", "Welcome.", &pcm(100)).unwrap();
    assert!(takes.restore("welcome").unwrap());
    assert!(takes.current("welcome", "Welcome.").is_none());
    assert!(Takes::load(&dir).unwrap().is_empty());
    assert!(!dir.join("welcome.wav").exists());
}

/// What is put aside is not a take: loading the directory sees one per line.
#[test]
fn a_take_put_aside_is_not_listed() {
    let dir = teleprompt_testkit::test_dir("takes-aside");
    let mut takes = Takes::load(&dir).unwrap();
    takes.save("welcome", "Welcome.", &pcm(100)).unwrap();
    takes.save("welcome", "Welcome.", &pcm(200)).unwrap();
    assert_eq!(Takes::load(&dir).unwrap().iter().count(), 1);
}
