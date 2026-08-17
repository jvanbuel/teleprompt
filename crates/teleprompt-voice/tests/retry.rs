//! What the retry policy promises `dub`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use teleprompt_voice::retry::{jitter_fraction, seed_from};
use teleprompt_voice::{with_retry, ErrorKind, RetryPolicy, VoiceError};

fn policy() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 4,
        base_delay: Duration::from_millis(500),
        max_delay: Duration::from_secs(30),
    }
}

#[tokio::test(start_paused = true)]
async fn succeeds_without_sleeping_when_the_first_attempt_works() {
    let r = with_retry(&policy(), 7, |_| async { Ok::<_, VoiceError>(42) }).await;
    assert_eq!(r.unwrap(), 42);
}

#[tokio::test(start_paused = true)]
async fn retries_a_transient_failure_then_succeeds() {
    let calls = AtomicU32::new(0);
    let r = with_retry(&policy(), 7, |_| {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            if n < 2 {
                Err(VoiceError::new("x", ErrorKind::Transient, "flaky"))
            } else {
                Ok(99)
            }
        }
    })
    .await;
    assert_eq!(r.unwrap(), 99);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

/// Quota is the whole reason the taxonomy exists: it arrives as a distinct
/// status from a rate limit precisely because waiting does not help, and
/// retrying it would spend four attempts restating that the account is out
/// of credit.
#[tokio::test(start_paused = true)]
async fn does_not_retry_a_fatal_failure() {
    let calls = AtomicU32::new(0);
    let r: Result<(), _> = with_retry(&policy(), 7, |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Err(VoiceError::new("x", ErrorKind::Quota, "out of credits")) }
    })
    .await;
    assert_eq!(r.unwrap_err().kind, ErrorKind::Quota);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "Quota must not be retried");
}

#[tokio::test(start_paused = true)]
async fn gives_up_after_max_attempts_and_returns_the_last_error() {
    let calls = AtomicU32::new(0);
    let r: Result<(), _> = with_retry(&policy(), 7, |n| {
        calls.fetch_add(1, Ordering::SeqCst);
        async move {
            Err(VoiceError::new(
                "x",
                ErrorKind::Transient,
                format!("attempt {n}"),
            ))
        }
    })
    .await;
    let e = r.unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert_eq!(
        e.detail, "attempt 3",
        "the last error is the one returned — it is the most recent \
         description of what the server is actually doing"
    );
}

/// A server that says how long to wait is more informative than our own
/// backoff curve, so its instruction wins.
#[tokio::test(start_paused = true)]
async fn a_server_supplied_retry_after_is_honoured() {
    let start = tokio::time::Instant::now();
    let calls = AtomicU32::new(0);
    let _: Result<(), _> = with_retry(&policy(), 7, |_| {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            Err(
                VoiceError::new("x", ErrorKind::RateLimited, format!("busy {n}"))
                    .with_retry_after(Duration::from_secs(5)),
            )
        }
    })
    .await;
    // Three sleeps of five seconds between four attempts. The computed
    // backoff would have been well under a second each, so this only holds
    // if `retry_after` overrode it.
    assert_eq!(start.elapsed(), Duration::from_secs(15));
}

/// `retry_after` must not be able to park a run for longer than the policy's
/// own ceiling, however enthusiastic the server is.
#[tokio::test(start_paused = true)]
async fn a_retry_after_beyond_max_delay_is_capped() {
    let start = tokio::time::Instant::now();
    let p = RetryPolicy {
        max_attempts: 2,
        base_delay: Duration::from_millis(500),
        max_delay: Duration::from_secs(30),
    };
    let _: Result<(), _> = with_retry(&p, 7, |_| async {
        Err(VoiceError::new("x", ErrorKind::RateLimited, "busy")
            .with_retry_after(Duration::from_secs(86_400)))
    })
    .await;
    assert_eq!(start.elapsed(), Duration::from_secs(30));
}

#[test]
fn jitter_is_deterministic_and_spread() {
    // Same seed and attempt always gives the same fraction. This is what
    // makes a retry test reproducible rather than usually-passing.
    assert_eq!(jitter_fraction(42, 1), jitter_fraction(42, 1));
    // Different segments decorrelate, which is the entire purpose.
    assert_ne!(jitter_fraction(42, 1), jitter_fraction(43, 1));
    // Successive attempts of one segment do not repeat either.
    assert_ne!(jitter_fraction(42, 1), jitter_fraction(42, 2));
    for seed in 0..1000u64 {
        let f = jitter_fraction(seed, 2);
        assert!((0.0..1.0).contains(&f), "fraction out of range: {f}");
    }
}

/// The spread has to actually spread: a "random" function that clusters in
/// one tenth of its range would decorrelate nothing.
#[test]
fn jitter_covers_its_range() {
    let mut buckets = [0usize; 10];
    for seed in 0..10_000u64 {
        let f = jitter_fraction(seed, 0);
        buckets[(f * 10.0) as usize] += 1;
    }
    for (i, n) in buckets.iter().enumerate() {
        assert!(*n > 700, "bucket {i} had only {n} of 10000");
    }
}

#[test]
fn seeds_are_stable_and_distinct() {
    assert_eq!(seed_from("seg-1"), seed_from("seg-1"));
    assert_ne!(seed_from("seg-1"), seed_from("seg-2"));
    // Pinned literal: FNV-1a is chosen over DefaultHasher precisely because
    // it is stable across Rust releases, and a test that would not notice a
    // change is not testing that.
    assert_eq!(seed_from(""), 0xcbf2_9ce4_8422_2325);
}
