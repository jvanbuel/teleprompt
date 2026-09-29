use teleprompt_core::{DurationSource, Hash};
use teleprompt_manifest::captions::{cues, srt, vtt, Cue};
use teleprompt_manifest::{AudioInfo, LineEntry, NarrationManifest, WordEntry, MANIFEST_VERSION};

fn line(start_ms: u64, duration_ms: u64, text: &str) -> LineEntry {
    LineEntry {
        id: "l".to_string(),
        text: text.to_string(),
        chapter: "intro".to_string(),
        start_ms,
        duration_ms,
        duration_source: DurationSource::Measured,
        audio: "audio/l.wav".to_string(),
        source_hash: Hash::of(text.as_bytes()),
        audio_hash: Hash::of(b""),
        words: None,
        tempo_permille: None,
    }
}

fn manifest(lines: Vec<LineEntry>) -> NarrationManifest {
    NarrationManifest {
        manifest_version: MANIFEST_VERSION,
        script: "tour.md".to_string(),
        locale: "en".to_string(),
        generated_by: "test".to_string(),
        duration_ms: 60_000,
        audio: AudioInfo {
            format: "wav".into(),
            sample_rate: 48_000,
            channels: 1,
        },
        chapters: Vec::new(),
        lines,
        shots: Vec::new(),
    }
}

/// Words one every `step` ms from the line's start, relative to it.
fn timed(text: &str, step: u64) -> Vec<WordEntry> {
    text.split_whitespace()
        .enumerate()
        .map(|(i, w)| WordEntry {
            text: w.to_string(),
            start_ms: i as u64 * step,
            end_ms: i as u64 * step + step - 50,
        })
        .collect()
}

/// A short line is one cue, shown while it is spoken.
#[test]
fn a_short_line_is_one_cue() {
    let c = cues(&manifest(vec![line(1500, 2000, "Let's see what is here.")]));
    assert_eq!(
        c,
        [Cue {
            start_ms: 1500,
            end_ms: 3500,
            rows: vec!["Let's see what is here.".to_string()]
        }]
    );
}

/// Two rows of at most 42 characters, broken at spaces, as subtitles are
/// read.
#[test]
fn a_cue_has_at_most_two_rows_of_42() {
    let text = "Deployment is one command, and it streams its progress as it goes.";
    let c = cues(&manifest(vec![line(0, 4000, text)]));
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].rows.len(), 2);
    assert!(c[0].rows.iter().all(|r| r.len() <= 42), "{:?}", c[0].rows);
    assert_eq!(c[0].rows.join(" "), text);
}

/// A line too long for one cue is split, preferably after a sentence, and
/// each part is shown when its words are said.
#[test]
fn a_long_line_is_split_at_its_words_times() {
    let text = "Welcome to Acme. Let me show you around the dashboard today. \
                Deployment is one command, and it streams its progress as it goes.";
    let mut l = line(10_000, 9000, text);
    l.words = Some(timed(text, 400));
    let c = cues(&manifest(vec![l]));
    assert!(c.len() >= 2, "{c:?}");
    assert_eq!(
        c[0].rows.join(" "),
        "Welcome to Acme. Let me show you around the dashboard today."
    );
    // "Deployment" is the 12th word: 11 * 400 ms in.
    assert_eq!(c[1].start_ms, 10_000 + 11 * 400);
    assert_eq!(c[0].end_ms, c[1].start_ms);
    let all: Vec<String> = c.iter().map(|q| q.rows.join(" ")).collect();
    assert_eq!(all.join(" "), text);
}

/// Without word timings, a line's time is shared by the length of its parts.
#[test]
fn without_word_times_parts_share_the_line_by_length() {
    let first = "a".repeat(40) + ". ";
    let text = format!("{first}{}", "b ".repeat(60).trim_end());
    let c = cues(&manifest(vec![line(0, 10_000, &text)]));
    assert!(c.len() >= 2);
    assert_eq!(c.first().unwrap().start_ms, 0);
    assert_eq!(c.last().unwrap().end_ms, 10_000);
    assert!(c.windows(2).all(|w| w[0].end_ms == w[1].start_ms));
}

/// SRT numbers its cues and writes times with a comma; WebVTT has a header
/// and a full stop.
#[test]
fn srt_and_vtt_are_written_as_players_expect() {
    let c = vec![
        Cue {
            start_ms: 1500,
            end_ms: 3500,
            rows: vec!["Let's see.".into()],
        },
        Cue {
            start_ms: 3_723_004,
            end_ms: 3_725_000,
            rows: vec!["One".into(), "two.".into()],
        },
    ];
    assert_eq!(
        srt(&c),
        "1\n00:00:01,500 --> 00:00:03,500\nLet's see.\n\n\
         2\n01:02:03,004 --> 01:02:05,000\nOne\ntwo.\n"
    );
    assert_eq!(
        vtt(&c),
        "WEBVTT\n\n00:00:01.500 --> 00:00:03.500\nLet's see.\n\n\
         01:02:03.004 --> 01:02:05.000\nOne\ntwo.\n"
    );
}

/// Rows are measured in characters, so accented text fills them as fully.
#[test]
fn rows_count_characters_not_bytes() {
    let text = "Déploiement: une seule commande, qui affiche où elle en est.";
    assert_eq!(text.chars().count(), 60);
    let c = cues(&manifest(vec![line(0, 4000, text)]));
    assert_eq!(c.len(), 1, "{c:?}");
    assert!(c[0].rows.iter().all(|r| r.chars().count() <= 42));
}
