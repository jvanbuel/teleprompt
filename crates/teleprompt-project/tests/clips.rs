//! The clip cache: a clip is there only once its session finished, under
//! the name a renderer looks for.

use std::path::Path;

use teleprompt_core::Hash;
use teleprompt_manifest::{clip_file, NarrationManifest};
use teleprompt_project::clips::ClipStore;
use teleprompt_scene::capture::Clip;

/// Writes each key's clip in the staging directory it is given.
fn writes(keys: &[Hash]) -> impl FnOnce(&Path) -> Result<Vec<Clip>, String> + '_ {
    move |staging| {
        Ok(keys
            .iter()
            .map(|key| {
                let path = staging.join(format!("{key}.mp4"));
                std::fs::write(&path, b"a clip").unwrap();
                Clip { key: *key, path }
            })
            .collect())
    }
}

fn store(dir: &teleprompt_testkit::TestDir) -> ClipStore {
    ClipStore::at(dir.path().join("video"))
}

/// A clip in the cache is taken as captured, so a failed session must
/// leave nothing there.
#[test]
fn a_failed_recording_files_nothing() {
    let dir = teleprompt_testkit::test_dir("clips-fail");
    let clips = store(&dir);
    let key = Hash::of(b"a#0");

    let e = clips
        .record(|staging| {
            writes(&[key])(staging)?;
            Err("interrupted".into())
        })
        .unwrap_err();

    assert_eq!(e, "interrupted");
    assert!(!clips.has(&key));
    assert_eq!(clips.path_of(&key), None);
    let left: Vec<_> = std::fs::read_dir(clips.dir()).unwrap().flatten().collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}

#[test]
fn a_finished_recording_files_every_clip_under_its_key() {
    let dir = teleprompt_testkit::test_dir("clips-ok");
    let clips = store(&dir);
    let keys = [Hash::of(b"a#0"), Hash::of(b"a#1")];

    assert_eq!(clips.record(writes(&keys)), Ok(2));

    for key in &keys {
        assert_eq!(
            clips.path_of(key),
            Some(clips.dir().join(clip_file(key))),
            "filed under the name a renderer looks for"
        );
    }
    let names: Vec<String> = std::fs::read_dir(clips.dir())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "no staging left behind: {names:?}");
}

/// Two shots, one of them a pause, which owes no clip.
fn manifest(shot: &Hash, pause: &Hash) -> NarrationManifest {
    let entry = |name: &str, scene: &str, key: &Hash| {
        serde_json::json!({
            "shot": name, "line": null, "scene": scene, "plugin": "vhs",
            "start_ms": 0, "duration_ms": 1000, "duration_source": "exact",
            "policy": "hold", "transition": { "kind": "cut", "duration_ms": 0 },
            "shot_hash": key.to_string(), "capture_key": key.to_string(),
        })
    };
    serde_json::from_value(serde_json::json!({
        "manifest_version": 3, "script": "tour.md", "locale": "en",
        "generated_by": "teleprompt", "duration_ms": 2000,
        "audio": { "format": "wav", "sample_rate": 24000, "channels": 1 },
        "chapters": [], "lines": [],
        "shots": [entry("a#0", "terminal", shot), entry("a#1", "pause", pause)],
    }))
    .expect("a manifest")
}

#[test]
fn placing_puts_the_manifests_clips_and_nothing_else() {
    let dir = teleprompt_testkit::test_dir("clips-place");
    let clips = store(&dir);
    let (shot, pause, other) = (Hash::of(b"a#0"), Hash::of(b"a#1"), Hash::of(b"b#0"));
    clips.record(writes(&[shot, other])).unwrap();
    let into = dir.path().join("out/clips");
    std::fs::create_dir_all(&into).unwrap();
    std::fs::write(into.join("stale.mp4"), b"from an earlier dub").unwrap();

    let placed = clips.place_into(&manifest(&shot, &pause), &into).unwrap();

    assert_eq!(placed, vec![into.join(clip_file(&shot))]);
    let names: Vec<String> = std::fs::read_dir(&into)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![clip_file(&shot)]);
}
