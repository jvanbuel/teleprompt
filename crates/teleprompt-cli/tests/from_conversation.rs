//! `from` with the conversation's recording: each line speaks its stretch
//! of it, in its speaker's own voice, until it is reworded.

use teleprompt_cli::cmd::from::{run_from, Audio, Reading};
use teleprompt_cli::project::Project;
use teleprompt_voice::takes::Takes;
use teleprompt_voice::{wav, Pcm};

const RATE: u32 = 16_000;

const VTT: &str = "WEBVTT\n\n\
    00:00:00.500 --> 00:00:02.000\n<v Ada Lovelace>The engine weaves algebraic patterns.\n\n\
    00:00:03.000 --> 00:00:04.000\n<v Charles Babbage>Just as the loom weaves flowers.\n\n\
    00:00:05.000 --> 00:00:06.000\n<v Ada Lovelace>Quite.\n";

/// Seven seconds, each second a level of its own, so where a take was
/// cut from shows in its samples.
fn recording() -> Pcm {
    Pcm {
        sample_rate: RATE,
        channels: 1,
        samples: (0..7 * RATE as usize)
            .map(|i| (i / RATE as usize) as i16 * 1000)
            .collect(),
    }
}

fn seconds(pcm: &Pcm) -> Vec<i16> {
    let mut levels: Vec<i16> = pcm.samples.iter().map(|s| s / 1000).collect();
    levels.dedup();
    levels
}

#[test]
fn each_line_is_given_its_stretch_of_the_recording() {
    let dir = teleprompt_testkit::test_dir("from-conversation");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let doc = dir.join("talk.vtt");
    std::fs::write(&doc, VTT).unwrap();
    let audio = dir.join("talk.wav");
    std::fs::write(&audio, wav::encode(&recording())).unwrap();
    let script = dir.join("scripts/talk.md");

    let audio = Audio {
        path: &audio,
        revoice: &[],
    };
    let report = run_from(&doc, Some(script.clone()), Reading::Document, Some(audio)).unwrap();
    assert_eq!(report.takes, 3, "{}", report.render());

    let project = Project::discover(&dir).unwrap();
    let takes = Takes::load(&project.takes_dir()).unwrap();
    let take = |id: &str| wav::decode(&takes.read(id).unwrap()).unwrap();
    // The cue and a little either side, never past halfway to the next:
    // 0.35 s to 2.25 s.
    let first = take("the-engine-weaves");
    assert_eq!(seconds(&first), [0, 1, 2]);
    assert_eq!(first.samples.len(), RATE as usize * 19 / 10);
    assert_eq!(seconds(&take("just-as-the")), [2, 3, 4]);
    assert_eq!(seconds(&take("quite")), [4, 5, 6]);
    assert!(!takes.stale("quite", "Quite."));
}

#[test]
fn a_transcript_with_no_times_cannot_be_heard() {
    let dir = teleprompt_testkit::test_dir("from-untimed");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let doc = dir.join("talk.txt");
    std::fs::write(&doc, "Ada: Hello.\n\nCharles: Hi.\n").unwrap();
    let audio = dir.join("talk.wav");
    std::fs::write(&audio, wav::encode(&recording())).unwrap();
    let script = dir.join("scripts/talk.md");
    let audio = Audio {
        path: &audio,
        revoice: &[],
    };
    let err = run_from(&doc, Some(script.clone()), Reading::Transcript, Some(audio)).unwrap_err();
    assert!(err.to_string().contains("turn 1"), "{err}");
    assert!(!script.exists(), "nothing is written");
}

#[test]
fn a_revoiced_speaker_is_left_to_their_voice_in_the_cast() {
    let dir = teleprompt_testkit::test_dir("from-revoice");
    teleprompt_cli::cmd::new::scaffold(&dir).unwrap();
    let doc = dir.join("talk.vtt");
    std::fs::write(&doc, VTT).unwrap();
    let audio = dir.join("talk.wav");
    std::fs::write(&audio, wav::encode(&recording())).unwrap();
    let revoice = ["Charles Babbage".to_string()];
    let audio = Audio {
        path: &audio,
        revoice: &revoice,
    };
    let script = dir.join("scripts/talk.md");
    let report = run_from(&doc, Some(script), Reading::Document, Some(audio)).unwrap();
    assert_eq!(report.takes, 2);
    let takes = Takes::load(&Project::discover(&dir).unwrap().takes_dir()).unwrap();
    let ids: Vec<&str> = takes.iter().map(|(id, _)| id).collect();
    assert_eq!(ids, ["quite", "the-engine-weaves"]);

    let nobody = ["Byron".to_string()];
    let audio = dir.join("talk.wav");
    let err = run_from(
        &doc,
        Some(dir.join("scripts/again.md")),
        Reading::Document,
        Some(Audio {
            path: &audio,
            revoice: &nobody,
        }),
    )
    .unwrap_err();
    assert!(err.to_string().contains("byron"), "{err}");
}
