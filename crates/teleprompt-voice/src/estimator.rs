use crate::contract::SynthRequest;

/// Predicts how long text takes to speak *without synthesizing it*, which
/// keeps the inner loop synchronous (`docs/design.md#async-boundary`).
///
/// Must be deterministic, or committed timelines churn.
pub trait DurationEstimator: Send + Sync {
    fn estimate_ms(&self, req: &SynthRequest) -> u64;
}
