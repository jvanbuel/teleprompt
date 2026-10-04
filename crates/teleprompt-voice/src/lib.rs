//! The voice engine: what speaks a line when no plugin does (`null`), how
//! long a line is expected to take before it is spoken, stretching a line
//! to fit, and the author's own takes. The contract a voice backend
//! implements is `teleprompt_plugin::voice`.

pub mod estimator;
pub mod null;
pub mod registry;
pub mod stretch;
pub mod takes;

pub use estimator::{DurationEstimator, WpmEstimator, DEFAULT_WPM};
pub use null::{NullVoice, NULL_SAMPLE_RATE};
pub use registry::VoiceRegistry;
