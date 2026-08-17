//! Retry policy for voice synthesis.
//!
//! This lives here and `dub` calls it — backends classify their failures
//! and never loop. A retry loop inside a client is invisible to `dub`'s
//! progress output, so the author sees a stalled segment with no
//! explanation, and it fights the semaphore above it by holding a permit
//! while it sleeps.

use std::future::Future;
use std::time::Duration;

use crate::contract::VoiceError;

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Total attempts, not retries after the first: `1` means never retry.
    pub max_attempts: u32,
    pub base_delay: Duration,
    /// Ceiling on any single sleep, including one a server asked for.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// A deterministic fraction in `[0, 1)` from a seed and an attempt number.
///
/// Full jitter needs a spread, not randomness as such: its purpose is to
/// decorrelate concurrent retries so a fan-out of segments does not retry in
/// lockstep and rebuild the same thundering herd that caused the rate limit.
/// Deriving the spread from the segment's own identity achieves that while
/// keeping the suite reproducible — a test that reruns a 429 storm gets the
/// same schedule twice, which is the difference between pinning behaviour
/// and usually passing. It also avoids a `rand` dependency.
///
/// splitmix64's finalizer. Not cryptographic, and does not need to be.
pub fn jitter_fraction(seed: u64, attempt: u32) -> f64 {
    let mut z = seed.wrapping_add((attempt as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // 53 bits is f64's mantissa, so every value here is exact.
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// A stable seed for a string identity, so the same segment jitters the same
/// way on every run.
///
/// FNV-1a: six lines, no dependency, and — unlike `DefaultHasher`, whose
/// output is explicitly not guaranteed stable across Rust releases — fixed
/// forever, which a test asserting a delay schedule depends on.
pub fn seed_from(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Run `op` until it succeeds, fails un-retryably, or runs out of attempts.
///
/// `op` receives the zero-based attempt number, so a caller can report a
/// retry to the author rather than leaving a segment apparently stalled.
///
/// The error returned on exhaustion is the *last* one, not the first: it is
/// the most recent description of what the server is actually doing, and a
/// first error preserved through three more failures would describe a state
/// that has since changed.
pub async fn with_retry<T, F, Fut>(
    policy: &RetryPolicy,
    seed: u64,
    mut op: F,
) -> Result<T, VoiceError>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T, VoiceError>>,
{
    let mut attempt = 0u32;
    loop {
        match op(attempt).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if attempt + 1 >= policy.max_attempts || !e.kind.retryable() {
                    return Err(e);
                }
                let delay = match e.retry_after {
                    // The server's own instruction beats our curve — but not
                    // the ceiling, or a server could park a run indefinitely.
                    Some(d) => d.min(policy.max_delay),
                    None => {
                        let backoff = policy
                            .base_delay
                            .saturating_mul(1u32 << attempt.min(16))
                            .min(policy.max_delay);
                        backoff.mul_f64(jitter_fraction(seed, attempt))
                    }
                };
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}
