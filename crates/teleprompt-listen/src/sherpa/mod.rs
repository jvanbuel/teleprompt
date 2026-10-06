//! A [`Recognizer`](crate::Recognizer) backed by a streaming sherpa-onnx
//! model, and the offline models a recording is drafted with: whole-file
//! transcription, punctuation and telling voices apart. Behind the opt-in
//! `sherpa` feature, so the default build stays offline.

mod diarize;
mod punctuation;
mod recognizer;

pub use diarize::diarize;
pub use punctuation::punctuate;
pub use recognizer::{transcribe, SherpaRecognizer, SAMPLE_RATE};
