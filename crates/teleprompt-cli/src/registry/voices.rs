//! The voices this build ships, each with what it needs.

use teleprompt_voices::{elevenlabs, gemini, openai, voicebox};

use super::needs;
use super::Voice;

/// `null` aside, in the order errors and `setup` list them.
pub fn shipped() -> Vec<Voice> {
    vec![
        (openai::kokoro(), &needs::KOKORO),
        (openai::openai(), &needs::OPENAI),
        (voicebox::provider(), &needs::VOICEBOX),
        (gemini::provider(), &needs::GEMINI),
        (elevenlabs::provider(), &needs::ELEVENLABS),
    ]
}
