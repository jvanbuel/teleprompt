//! What this plugin needs that teleprompt does not ship, as
//! `teleprompt setup` lists and installs it.

use teleprompt_plugin::tool::{Found, Tool};

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
         names). Any other server that speaks its API is `[backends.<name>] api = \"openai\"`: \
         see docs/guide/voices.md.",
    ),
    found: Found::Unknowable,
    install: &[],
    download_mb: None,
};
