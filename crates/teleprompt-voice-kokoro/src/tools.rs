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
