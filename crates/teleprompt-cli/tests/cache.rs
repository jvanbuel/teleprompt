//! The caches a project keeps, and how they are kept from growing for ever.
//!
//! Everything here is arithmetic over a directory of files, so none of it
//! needs ffmpeg, a voice backend, or a script.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use teleprompt::cache;

/// A cache entry of `bytes` bytes, last used `age` ago.
fn entry(dir: &Path, name: &str, bytes: usize, age: Duration) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, vec![b'x'; bytes]).unwrap();
    let when = SystemTime::now() - age;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(when))
        .unwrap();
    path
}

fn workdir(name: &str) -> teleprompt_testkit::TestDir {
    teleprompt_testkit::test_dir(&format!("cache-{name}"))
}

#[test]
fn a_directory_that_was_never_built_in_is_empty_rather_than_an_error() {
    let stats = cache::stats(&workdir("absent").join("compose"));
    assert_eq!(stats.entries, 0);
    assert_eq!(stats.bytes, 0);
}

/// A half-written entry is not an entry. The renderer encodes beside the
/// name it is going to use and renames into place, so a dotfile in the
/// cache is a render that was killed — it must not be counted, and it must
/// not be what a prune decides to evict instead of real data.
#[test]
fn a_partial_encode_is_not_counted_as_an_entry() {
    let dir = workdir("partial");
    entry(&dir, "aaaa.mp4", 100, Duration::ZERO);
    entry(&dir, ".bbbb.partial.mp4", 900, Duration::ZERO);

    let stats = cache::stats(&dir);
    assert_eq!(stats.entries, 1);
    assert_eq!(stats.bytes, 100);
}

/// The voice cache stores a WAV and a sidecar under one key. They are one
/// entry: evicting the audio and keeping the sidecar would leave a hit
/// with no bytes behind it.
#[test]
fn the_files_that_share_a_key_are_one_entry() {
    let dir = workdir("pairs");
    entry(&dir, "aaaa.wav", 100, Duration::ZERO);
    entry(&dir, "aaaa.json", 20, Duration::ZERO);

    let stats = cache::stats(&dir);
    assert_eq!(stats.entries, 1, "one key, one entry");
    assert_eq!(stats.bytes, 120, "and it costs what both files cost");

    cache::prune(&dir, 0).unwrap();
    assert_eq!(cache::stats(&dir).entries, 0);
    assert!(!dir.join("aaaa.json").exists(), "the sidecar went with it");
}

/// The point of the cap. Oldest goes first, and what a recent build
/// depended on stays.
#[test]
fn a_prune_evicts_the_least_recently_used_until_it_fits() {
    let dir = workdir("lru");
    entry(&dir, "old.mp4", 500, Duration::from_secs(3_600));
    entry(&dir, "older.mp4", 500, Duration::from_secs(7_200));
    entry(&dir, "fresh.mp4", 500, Duration::ZERO);

    let pruned = cache::prune(&dir, 1_000).unwrap();

    assert_eq!(pruned.removed, 1);
    assert_eq!(pruned.freed, 500);
    assert_eq!(pruned.bytes, 1_000, "what is left is under the cap");
    assert!(!dir.join("older.mp4").exists(), "the oldest went");
    assert!(dir.join("old.mp4").exists());
    assert!(dir.join("fresh.mp4").exists());
}

#[test]
fn a_cache_that_already_fits_is_left_alone() {
    let dir = workdir("fits");
    entry(&dir, "a.mp4", 500, Duration::from_secs(7_200));

    let pruned = cache::prune(&dir, 1_000).unwrap();
    assert_eq!(pruned.removed, 0);
    assert_eq!(pruned.bytes, 500);
    assert!(dir.join("a.mp4").exists());
}

/// A cap of nothing is a legitimate thing to ask for — it is how you get
/// the disk back — and it is what `--max-mb 0` means.
#[test]
fn a_cap_of_nothing_keeps_nothing() {
    let dir = workdir("zero");
    entry(&dir, "a.mp4", 500, Duration::ZERO);
    entry(&dir, "b.mp4", 500, Duration::ZERO);

    let pruned = cache::prune(&dir, 0).unwrap();
    assert_eq!(pruned.removed, 2);
    assert_eq!(pruned.freed, 1_000);
    assert_eq!(pruned.bytes, 0);
}

/// Pruning a directory nothing has written to is not a failure; it is a
/// project that has never built.
#[test]
fn pruning_a_cache_that_does_not_exist_does_nothing() {
    let pruned = cache::prune(&workdir("nodir").join("compose"), 1_000).unwrap();
    assert_eq!(pruned.removed, 0);
    assert_eq!(pruned.bytes, 0);
}
