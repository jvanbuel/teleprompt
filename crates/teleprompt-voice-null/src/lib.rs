//! The reference voice backend: silence of exactly the estimated length,
//! and the word-count duration model behind it.
//!
//! This lives outside `teleprompt-voice` deliberately. If the reference
//! implementation cannot be written from outside the contract crate using
//! only its public API, the contract is not a contract.

pub mod estimator;
mod null;

pub use estimator::{WpmEstimator, DEFAULT_WPM};
pub use null::{NullVoice, NULL_SAMPLE_RATE};
