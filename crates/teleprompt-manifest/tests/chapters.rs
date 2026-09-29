use teleprompt_core::{SpanMs, TimeMs};
use teleprompt_manifest::chapters::youtube;
use teleprompt_manifest::{AudioInfo, ChapterEntry, NarrationManifest, MANIFEST_VERSION};

fn manifest(chapters: &[(&str, u64)], duration_ms: u64) -> NarrationManifest {
    NarrationManifest {
        manifest_version: MANIFEST_VERSION,
        script: "tour.md".into(),
        locale: "en".into(),
        generated_by: "test".into(),
        duration_ms: SpanMs::of(duration_ms),
        audio: AudioInfo {
            format: "wav".into(),
            sample_rate: 48_000,
            channels: 1,
        },
        chapters: chapters
            .iter()
            .map(|(title, start_ms)| ChapterEntry {
                id: title.to_lowercase(),
                title: (*title).into(),
                start_ms: TimeMs::at(*start_ms),
            })
            .collect(),
        lines: Vec::new(),
        shots: Vec::new(),
    }
}

/// One line per chapter, as YouTube reads a description: the first at
/// 0:00, times rounded down to the second.
#[test]
fn chapters_are_listed_as_youtube_reads_them() {
    let m = manifest(
        &[
            ("Introduction", 150),
            ("Configuration", 72_900),
            ("Deploying", 3_723_000),
        ],
        3_800_000,
    );
    let (text, problems) = youtube(&m);
    assert_eq!(
        text,
        "0:00 Introduction\n1:12 Configuration\n1:02:03 Deploying\n"
    );
    assert!(problems.is_empty(), "{problems:?}");
}

/// A list YouTube would ignore says why, rather than being ignored
/// silently.
#[test]
fn a_list_youtube_would_ignore_says_why() {
    let (_, problems) = youtube(&manifest(&[("One", 0), ("Two", 30_000)], 60_000));
    assert!(
        problems.iter().any(|p| p.contains("at least three")),
        "{problems:?}"
    );

    let (_, problems) = youtube(&manifest(
        &[("One", 0), ("Two", 30_000), ("Three", 34_000)],
        60_000,
    ));
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`Two`") && p.contains("10 seconds")),
        "{problems:?}"
    );
}
