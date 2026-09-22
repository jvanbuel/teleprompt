use std::sync::{Arc, Barrier};

use teleprompt_cache::{key, CacheKey, CachedMeta, VoiceCache};
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
    assert!(c.lookup(&k).unwrap().hit().is_none());
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

    let got = c.lookup(&k).unwrap().hit().expect("hit");
    assert_eq!(got.duration_ms, 1000);
    assert_eq!(got.sample_rate, 24_000);
    assert_eq!(got.channels, 1);
    assert_eq!(got.wav, stored.wav);
    assert!(got.word_timings.is_none());
}

/// `compile` runs on the inner loop and never needs the WAV; `lookup_meta`
/// is how it avoids reading audio back only to drop it.
#[test]
fn lookup_meta_round_trips_without_the_wav() {
    let c = VoiceCache::new(tempdir("meta-round"));
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));
    c.store(&k, &pcm(1000), None).unwrap();

    let got = c.lookup_meta(&k).unwrap().hit().expect("hit");
    assert_eq!(
        got,
        CachedMeta {
            duration_ms: 1000,
            sample_rate: 24_000,
            channels: 1,
            word_timings: None,
        }
    );
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
        c.lookup(&k).unwrap().hit().unwrap().word_timings.unwrap(),
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

/// I4. A corrupt sidecar used to be an error, which made a gitignored,
/// entirely derived artifact fail `check` — a validation command — with the
/// blame attributed to the script and no recovery offered. The cache is
/// content-addressed: an unreadable entry is missing information, never
/// wrong information, so the only correct reading is a miss. The warning is
/// what keeps that from being silent, since a line would otherwise revert
/// from `measured` to `estimated` for no visible reason.
#[test]
fn a_corrupt_sidecar_reads_as_a_miss_and_names_itself() {
    let root = tempdir("corrupt");
    let c = VoiceCache::new(&root);
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));
    c.store(&k, &pcm(1000), None).unwrap();

    std::fs::write(root.join(format!("voice/{k}.json")), "{ not json").unwrap();

    for (what, read) in [
        ("lookup", c.lookup(&k).unwrap().warning()),
        ("lookup_meta", c.lookup_meta(&k).unwrap().warning()),
    ] {
        let w = read.unwrap_or_else(|| panic!("{what} must warn about a corrupt entry"));
        assert!(w.contains(&k.to_string()), "{what}: {w}");
    }

    assert!(
        c.lookup(&k).unwrap().hit().is_none(),
        "a corrupt entry must re-synthesize, not error"
    );
    assert!(c.lookup_meta(&k).unwrap().hit().is_none());
}

/// An ordinary miss has nothing to explain, so it must not manufacture a
/// warning — a `plan` on a cold project would otherwise print one per
/// line.
#[test]
fn an_ordinary_miss_carries_no_warning() {
    let root = tempdir("quietmiss");
    let c = VoiceCache::new(&root);
    let k = key("null", "0.1.0", &req("never stored", None, 1.0, "en"));
    assert_eq!(c.lookup(&k).unwrap().warning(), None);
    assert_eq!(c.lookup_meta(&k).unwrap().warning(), None);
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
        c.lookup(&k).unwrap().hit().is_none(),
        "treat as absent and re-synthesize"
    );
}

/// Mirrors `a_sidecar_without_its_wav_is_a_miss`: `lookup_meta` must not
/// report a hit off the sidecar alone, even though it never reads the WAV
/// it's checking for.
#[test]
fn lookup_meta_of_a_sidecar_without_its_wav_is_a_miss() {
    let root = tempdir("meta-halfwritten");
    let c = VoiceCache::new(&root);
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));
    c.store(&k, &pcm(1000), None).unwrap();

    std::fs::remove_file(root.join(format!("voice/{k}.wav"))).unwrap();
    assert!(
        c.lookup_meta(&k).unwrap().hit().is_none(),
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

/// `stats` counted `.wav` files, so a WAV whose sidecar was lost — an
/// interrupted `dub`, a half-deleted cache — showed up in `doctor` as an
/// entry. `lookup` keys off the sidecar and reads that same pair as a miss,
/// so the count was reporting something the cache would not serve.
#[test]
fn an_orphaned_wav_is_not_counted_as_an_entry() {
    let root = tempdir("orphan");
    let c = VoiceCache::new(&root);
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));
    c.store(&k, &pcm(1000), None).unwrap();
    assert_eq!(c.stats().unwrap().entries, 1);

    std::fs::remove_file(root.join(format!("voice/{k}.json"))).unwrap();

    let s = c.stats().unwrap();
    assert_eq!(
        s.entries, 0,
        "an entry `lookup` reads as a miss is not an entry"
    );
    assert!(
        s.bytes > 0,
        "the bytes are still on disk, and a reader wondering where the space \
         went is owed that"
    );
}

/// The sidecar's `duration_ms` beside the byte length of the WAV actually on
/// disk. `pcm` is 24 kHz mono, so a WAV of `d` ms is exactly `44 + 48 * d`
/// bytes — a pair where these two disagree is one where the sidecar
/// describes audio other than the file next to it.
fn pair_on_disk(root: &std::path::Path, k: &CacheKey) -> (u64, u64) {
    let raw = std::fs::read_to_string(root.join(format!("voice/{k}.json"))).expect("sidecar");
    let side: serde_json::Value = serde_json::from_str(&raw).expect("sidecar parses");
    let duration_ms = side["duration_ms"].as_u64().expect("duration_ms");
    let wav_len = std::fs::metadata(root.join(format!("voice/{k}.wav")))
        .expect("wav")
        .len();
    (duration_ms, wav_len)
}

/// Two threads storing the same key with audio of different lengths, which
/// is what two concurrent `teleprompt dub` processes on one project do: a
/// real TTS server is not bit-deterministic, so two renders of one sentence
/// differ slightly in length and each process stores its own.
fn race_one_key(root: &std::path::Path, k: &CacheKey, a_ms: u64, b_ms: u64) -> Vec<u64> {
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [a_ms, b_ms]
        .into_iter()
        .map(|ms| {
            let root = root.to_path_buf();
            let k = k.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let c = VoiceCache::new(&root);
                let audio = pcm(ms);
                barrier.wait();
                c.store(&k, &audio, None).expect("store").duration_ms
            })
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).collect()
}

/// I3. `store` used to be two plain `std::fs::write` calls with nothing
/// serialising writers, so the interleaving `wav_A → wav_B → sidecar_B →
/// sidecar_A` left a well-formed sidecar beside audio it does not describe.
/// Nothing above this layer can detect that: on a cache *hit* the duration
/// `dub` publishes and the duration it checks the file against both come
/// from this one sidecar, so the WAV-length guard compares a value against
/// itself.
///
/// The audio is deliberately *small*. The dangerous interleaving needs each
/// writer's two writes to be pulled apart, which happens when the writes are
/// short enough that scheduling jitter dominates them — with ~1 MB payloads
/// the long WAV write swamps the gap and the pair almost always comes out
/// consistent by luck. At 1 ms and 2 ms of audio the pre-fix `store`
/// mismatches on 10–30% of rounds, so sixty-four rounds against sixty-four
/// distinct keys fails it every time (measured: ten runs, 6–19 bad pairs
/// each). No hook or fault injection — this is the real `store`.
#[test]
fn concurrent_stores_of_one_key_leave_a_consistent_pair() {
    let root = tempdir("race-pair");
    for round in 0..64 {
        let k = key(
            "null",
            "0.1.0",
            &req(&format!("race {round}"), None, 1.0, "en"),
        );
        race_one_key(&root, &k, 1, 2);

        let (duration_ms, wav_len) = pair_on_disk(&root, &k);
        assert_eq!(
            wav_len,
            44 + 48 * duration_ms,
            "round {round}: sidecar says {duration_ms}ms but the WAV beside it is \
             {wav_len} bytes"
        );
    }
}

/// The same race seen from the caller's side. `dub` uses what `store`
/// returns for the audio it writes out, then recompiles against the cache
/// and publishes the duration the *sidecar* holds — so a `store` that hands
/// back a length the published sidecar does not agree with makes `dub` fail
/// its own length guard and blame itself for an external race. Whoever wins
/// the key, both callers must be told the same thing the sidecar says.
#[test]
fn every_racing_store_returns_the_entry_that_was_published() {
    let root = tempdir("race-return");
    for round in 0..64 {
        let k = key(
            "null",
            "0.1.0",
            &req(&format!("race {round}"), None, 1.0, "en"),
        );
        let returned = race_one_key(&root, &k, 1, 2);

        let (duration_ms, _) = pair_on_disk(&root, &k);
        for got in returned {
            assert_eq!(
                got, duration_ms,
                "round {round}: store returned {got}ms while the cache holds \
                 {duration_ms}ms"
            );
        }
    }
}

/// A published entry is immutable: the key is a hash of everything that
/// changes the audio, so any complete entry under it is *the* answer, and a
/// later store of the same key adopts it rather than replacing it. This is
/// what stops a second writer from invalidating a duration the first writer
/// has already published in a manifest.
#[test]
fn storing_a_key_that_is_already_published_adopts_the_published_entry() {
    let root = tempdir("immutable");
    let c = VoiceCache::new(&root);
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));

    let first = c.store(&k, &pcm(1000), None).unwrap();
    let second = c.store(&k, &pcm(2000), None).unwrap();

    assert_eq!(second.duration_ms, first.duration_ms);
    assert_eq!(second.wav, first.wav);
    let (duration_ms, wav_len) = pair_on_disk(&root, &k);
    assert_eq!(duration_ms, 1000);
    assert_eq!(wav_len, 44 + 48 * 1000);
}

/// A WAV with no sidecar beside it is a half-written entry from an
/// interrupted `dub`. `lookup` reads it as a miss, so the next run
/// re-synthesizes — and `store` has to be able to replace it, or the entry
/// stays a permanent miss and the line is re-rendered on every run
/// forever.
#[test]
fn an_orphaned_wav_is_healed_by_the_next_store() {
    let root = tempdir("heal-orphan");
    let c = VoiceCache::new(&root);
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));

    c.store(&k, &pcm(1000), None).unwrap();
    std::fs::remove_file(root.join(format!("voice/{k}.json"))).unwrap();

    let healed = c.store(&k, &pcm(2000), None).unwrap();
    assert_eq!(healed.duration_ms, 2000);
    let (duration_ms, wav_len) = pair_on_disk(&root, &k);
    assert_eq!(duration_ms, 2000);
    assert_eq!(wav_len, 44 + 48 * 2000);
    assert_eq!(c.lookup(&k).unwrap().hit().expect("hit").duration_ms, 2000);
}

/// The corrupt-sidecar path, end to end: `lookup` reads it as a miss and
/// warns, and the next `store` must overwrite it rather than treating the
/// key as already published and adopting an entry nothing can parse.
#[test]
fn a_corrupt_sidecar_is_healed_by_the_next_store() {
    let root = tempdir("heal-corrupt");
    let c = VoiceCache::new(&root);
    let k = key("null", "0.1.0", &req("hello", None, 1.0, "en"));

    c.store(&k, &pcm(1000), None).unwrap();
    std::fs::write(root.join(format!("voice/{k}.json")), "{ not json").unwrap();

    let healed = c.store(&k, &pcm(2000), None).unwrap();
    assert_eq!(healed.duration_ms, 2000);
    let (duration_ms, wav_len) = pair_on_disk(&root, &k);
    assert_eq!(duration_ms, 2000);
    assert_eq!(wav_len, 44 + 48 * 2000);
}

/// A store leaves nothing behind but the pair itself. Temp files are named
/// per process and per call so two writers never share one, and both the
/// success and the failure paths remove them — a crashed run must not
/// litter a directory `doctor` reports the size of.
#[test]
fn a_store_leaves_no_temporary_files_behind() {
    let root = tempdir("no-litter");
    let c = VoiceCache::new(&root);
    for i in 0..4 {
        let k = key("null", "0.1.0", &req(&format!("t{i}"), None, 1.0, "en"));
        c.store(&k, &pcm(100), None).unwrap();
    }
    let names: Vec<String> = std::fs::read_dir(root.join("voice"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.ends_with(".wav") && !n.ends_with(".json"))
        .collect();
    assert!(names.is_empty(), "left behind: {names:?}");
}
