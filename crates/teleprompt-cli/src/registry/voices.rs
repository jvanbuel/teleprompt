//! The voices this build ships, each with what it needs.

use teleprompt_voices::{elevenlabs, gemini, openai, voicebox};

use super::needs;
use teleprompt_voice::catalogue::Shipped;

/// `null` aside, in the order errors and `setup` list them.
pub fn shipped() -> Vec<Shipped> {
    vec![
        (openai::kokoro(), &needs::KOKORO),
        (openai::openai(), &needs::OPENAI),
        (voicebox::provider(), &needs::VOICEBOX),
        (gemini::provider(), &needs::GEMINI),
        (elevenlabs::provider(), &needs::ELEVENLABS),
    ]
}
