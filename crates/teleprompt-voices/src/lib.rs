//! The voices teleprompt ships, one module each, all written against the
//! voice contract (`teleprompt-voice`) and nothing else of teleprompt's:
//! what a voice of the author's could do, these do.
//!
//! - [`openai`], any server that speaks OpenAI's speech API, with the
//!   `kokoro` and `openai` presets.
//! - [`voicebox`], a local Voicebox studio, in a voice cloned from takes.
//! - [`gemini`], Google's Gemini TTS.
//! - [`elevenlabs`], ElevenLabs.

pub mod elevenlabs;
pub mod gemini;
pub mod openai;
pub mod voicebox;
