//! A [`teleprompt_listen::Recognizer`] backed by a streaming sherpa-onnx
//! model, behind the `sherpa` feature.

#[cfg(feature = "sherpa")]
mod punctuation;
#[cfg(feature = "sherpa")]
mod sherpa;

#[cfg(feature = "sherpa")]
pub use punctuation::punctuate;
#[cfg(feature = "sherpa")]
pub use sherpa::{transcribe, SherpaRecognizer, SAMPLE_RATE};
