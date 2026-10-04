//! What this plugin needs that teleprompt does not ship, as
//! `teleprompt setup` lists and installs it.

use teleprompt_plugin::tool::{Found, Tool};

pub static ELEVENLABS: Tool = Tool {
    name: "elevenlabs",
    what: "speaks the narration, when `voice.backend` is \"elevenlabs\"",
    license: "a hosted service under ElevenLabs' terms (https://elevenlabs.io/terms-of-use)",
    home: "https://elevenlabs.io/docs/api-reference/text-to-speech/convert-with-timestamps",
    guide: Some(
        "ElevenLabs runs at ElevenLabs, not here, and the narration is sent there: \
         put an API key from elevenlabs.io/app/settings/api-keys in ELEVENLABS_API_KEY. \
         See docs/guide/voices.md.",
    ),
    found: Found::Unknowable,
    install: &[],
    download_mb: None,
};
