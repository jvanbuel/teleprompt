use teleprompt_cache::{key, VoiceCache};
use teleprompt_voice::{Pcm, SynthRequest, WordTiming};

fn req(text: &str, voice: Option<&str>, speed: f64, locale: &str) -> SynthRequest {
    SynthRequest {
        text: text.to_string(),
        locale: locale.to_string(),
        voice: voice.map(str::to_string),
        speed,
    }
}

fn pcm(ms: u64) -> Pcm {
    Pcm {
        sample_rate: 24_000,
        channels: 1,
        samples: vec![0; (ms * 24) as usize],
    }
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "tp-cache-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_miss_is_none_not_an_error() {
    let c = VoiceCache::new(tempdir("miss"));
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));
    assert!(c.lookup(&k).unwrap().is_none());
}

#[test]
fn store_then_lookup_round_trips() {
    let c = VoiceCache::new(tempdir("round"));
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));

    let stored = c.store(&k, &pcm(1000), None).unwrap();
    assert_eq!(stored.duration_ms, 1000);
    assert_eq!(stored.sample_rate, 24_000);
    assert_eq!(stored.channels, 1);
    assert_eq!(&stored.wav[0..4], b"RIFF");

    let got = c.lookup(&k).unwrap().expect("hit");
    assert_eq!(got.duration_ms, 1000);
    assert_eq!(got.sample_rate, 24_000);
    assert_eq!(got.channels, 1);
    assert_eq!(got.wav, stored.wav);
    assert!(got.word_timings.is_none());
}

#[test]
fn word_timings_survive_the_round_trip() {
    let c = VoiceCache::new(tempdir("words"));
    let k = key("null", "0.1.0", &req("hello there", None, 1.0, "en"));
    let timings = vec![
        WordTiming {
            word: "hello".into(),
            start_ms: 0,
            end_ms: 400,
        },
        WordTiming {
            word: "there".into(),
            start_ms: 400,
            end_ms: 900,
        },
    ];

    c.store(&k, &pcm(900), Some(&timings)).unwrap();
    assert_eq!(
        c.lookup(&k).unwrap().unwrap().word_timings.unwrap(),
        timings
    );
}

/// Every input that changes the audio must change the key. Varied one at a
/// time, because a key that ignores one field serves the wrong voice's audio
/// for another and nothing would ever notice.
#[test]
fn every_input_that_changes_the_audio_changes_the_key() {
    let base = key("null", "0.1.0", &req("hello", Some("af_heart"), 1.0, "en"));

    let variants = [
        (
            "backend id",
            key(
                "kokoro",
                "0.1.0",
                &req("hello", Some("af_heart"), 1.0, "en"),
            ),
        ),
        (
            "backend version",
            key("null", "0.2.0", &req("hello", Some("af_heart"), 1.0, "en")),
        ),
        (
            "text",
            key(
                "null",
                "0.1.0",
                &req("goodbye", Some("af_heart"), 1.0, "en"),
            ),
        ),
        (
            "voice",
            key("null", "0.1.0", &req("hello", Some("af_bella"), 1.0, "en")),
        ),
        (
            "no voice",
            key("null", "0.1.0", &req("hello", None, 1.0, "en")),
        ),
        (
            "speed",
            key("null", "0.1.0", &req("hello", Some("af_heart"), 2.0, "en")),
        ),
        (
            "locale",
            key("null", "0.1.0", &req("hello", Some("af_heart"), 1.0, "nl")),
        ),
    ];

    for (what, k) in variants {
        assert_ne!(
            base.to_string(),
            k.to_string(),
            "{what} must change the key"
        );
    }
}

#[test]
fn the_key_is_stable_across_calls() {
    let a = key("null", "0.1.0", &req("hello", Some("af_heart"), 1.0, "en"));
    let b = key("null", "0.1.0", &req("hello", Some("af_heart"), 1.0, "en"));
    assert_eq!(a.to_string(), b.to_string());
    assert_eq!(a.to_string().len(), 64, "64 hex characters");
}

#[test]
fn a_separator_inside_a_field_cannot_forge_another_field() {
    let a = key(
        "null",
        "0.1.0",
        &req("hello", Some("af_heart"), 1.0, "en/US"),
    );
    let b = key(
        "null",
        "0.1.0",
        &req("hello", Some("US/af_heart"), 1.0, "en"),
    );
    assert_ne!(
        a.to_string(),
        b.to_string(),
        "a `/` inside locale must not be able to impersonate the field boundary"
    );
}

#[test]
fn no_voice_is_distinguishable_from_a_voice_literally_named_dash() {
    let none = key("null", "0.1.0", &req("hello", None, 1.0, "en"));
    let dash = key("null", "0.1.0", &req("hello", Some("-"), 1.0, "en"));
    assert_ne!(none.to_string(), dash.to_string());
}

#[test]
fn a_corrupt_sidecar_is_an_error_not_a_panic() {
    let root = tempdir("corrupt");
    let c = VoiceCache::new(&root);
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));
    c.store(&k, &pcm(1000), None).unwrap();

    std::fs::write(root.join(format!("voice/{k}.json")), "{ not json").unwrap();
    assert!(c.lookup(&k).is_err());
}

/// A sidecar with no audio beside it is a half-written entry, not a hit.
#[test]
fn a_sidecar_without_its_wav_is_a_miss() {
    let root = tempdir("halfwritten");
    let c = VoiceCache::new(&root);
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));
    c.store(&k, &pcm(1000), None).unwrap();

    std::fs::remove_file(root.join(format!("voice/{k}.wav"))).unwrap();
    assert!(
        c.lookup(&k).unwrap().is_none(),
        "treat as absent and re-synthesize"
    );
}

#[test]
fn stats_count_entries_and_bytes() {
    let c = VoiceCache::new(tempdir("stats"));
    assert_eq!(c.stats().unwrap().entries, 0);

    c.store(
        &key("null", "0.1.0", &req("a", None, 1.0, "en")),
        &pcm(500),
        None,
    )
    .unwrap();
    c.store(
        &key("null", "0.1.0", &req("b", None, 1.0, "en")),
        &pcm(500),
        None,
    )
    .unwrap();

    let s = c.stats().unwrap();
    assert_eq!(s.entries, 2);
    assert!(s.bytes > 0);
}
