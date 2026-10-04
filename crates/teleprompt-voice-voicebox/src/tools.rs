//! What this plugin needs that teleprompt does not ship, as
//! `teleprompt setup` lists and installs it.

use teleprompt_plugin::tool::{Found, Tool};

pub static VOICEBOX: Tool = Tool {
    name: "voicebox",
    what:
        "speaks the narration in a cloned or designed voice, when `voice.backend` is \"voicebox\"",
    license: "MIT, the app; its models have their own: Qwen3-TTS, LuxTTS and Kokoro Apache-2.0, \
              Chatterbox MIT (its audio is watermarked), and TADA's weights the Llama 3.2 \
              Community License, which has conditions",
    home: "https://voicebox.sh",
    guide: Some(
        "Voicebox is an app you run yourself (from voicebox.sh, or in Docker); set \
         `voice.backend = \"voicebox\"` and make a voice from your takes with \
         `teleprompt voice clone <name>`: see docs/guide/voices.md.",
    ),
    found: Found::Unknowable,
    install: &[],
    download_mb: None,
};
