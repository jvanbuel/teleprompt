//! What this plugin needs that teleprompt does not ship, as
//! `teleprompt setup` lists and installs it.

use teleprompt_plugin::tool::{Found, Tool};

pub static GEMINI: Tool = Tool {
    name: "gemini",
    what: "speaks the narration, when `voice.backend` is \"gemini\": Google's Gemini 3.8 TTS",
    license: "a hosted service under Google's Gemini API terms \
              (https://ai.google.dev/gemini-api/terms); its audio carries a SynthID watermark",
    home: "https://ai.google.dev/gemini-api/docs/speech-generation",
    guide: Some(
        "Gemini runs at Google, not here, and the narration is sent there: \
         put an API key from aistudio.google.com/apikey in GEMINI_API_KEY. \
         See docs/guide/voices.md.",
    ),
    found: Found::Unknowable,
    install: &[],
    download_mb: None,
};
