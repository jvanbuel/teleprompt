//! What each voice teleprompt ships needs that it does not, as `teleprompt
//! setup` lists it: a server the author runs, or a key.

use teleprompt_scene::core::tool::{Found, Tool};

pub static KOKORO: Tool = Tool {
    name: "kokoro",
    what: "speaks the narration, when `voice.backend` is \"kokoro\"",
    license: "Apache-2.0, as are the Kokoro-82M weights",
    home: "https://github.com/remsky/Kokoro-FastAPI",
    guide: Some(
        "Kokoro is a server you run yourself, in Docker or with pip, \
         and `backends.kokoro.base_url` points at: see docs/guide/voices.md.",
    ),
    found: Found::Unknowable,
    install: &[],
    download_mb: None,
};

pub static OPENAI: Tool = Tool {
    name: "openai",
    what: "speaks the narration, when `voice.backend` is \"openai\"",
    license: "a paid service, under OpenAI's terms",
    home: "https://platform.openai.com/docs/guides/text-to-speech",
    guide: Some(
        "OpenAI's speech runs at OpenAI, not here, and the narration is sent there: \
         put an API key in OPENAI_API_KEY (or the variable `backends.openai.api_key_env` \
         names). Any other server that speaks its API is `[backends.<name>]` with its \
         `base_url`: see docs/guide/voices.md.",
    ),
    found: Found::Unknowable,
    install: &[],
    download_mb: None,
};

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
