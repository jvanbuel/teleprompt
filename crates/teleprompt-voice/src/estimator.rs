use crate::contract::SynthRequest;

/// Predicts how long text takes to speak *without synthesizing it*.
///
/// Separate from [`crate::VoiceBackend`] on purpose. A backend runs a model
/// and takes seconds; the inner loop (`plan`, `diff`) needs a duration for
/// every segment on every keystroke-driven run. Keeping prediction out of
/// the backend contract is what lets the backend be async while this stays
/// synchronous and free.
///
/// Implementations must be deterministic: the same request always yields the
/// same number, or committed timelines churn.
pub trait DurationEstimator: Send + Sync {
    fn estimate_ms(&self, req: &SynthRequest) -> u64;
}
