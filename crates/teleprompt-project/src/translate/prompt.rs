//! What every model-backed provider is told, and how its answer is read.

use serde_json::{json, Value};

use crate::translate::Response;

pub(crate) const SYSTEM: &str = "\
You translate the narration of a narrated software video. Each line is \
spoken aloud by a text-to-speech voice over a screen recording, so write \
what a native speaker would say: natural, spoken, and about as long as the \
original, since the pictures are timed to it.

Keep product names, commands, code, file names, flags and keys exactly as \
written. Keep the terminology of the translations already made. A chapter \
is a short heading.

A cue is a phrase of a line that a shot starts on. For each cue, answer \
with the words of your translation of that line (given by `line`, or in \
`existing` when it is not being translated now) that say the same thing, \
copied exactly, character for character. When the phrase is a command \
kept as written, answer with it unchanged.

Answer every item in `translate`, by its `id`, as JSON: \
{\"items\": [{\"id\": ..., \"text\": ...}]}.";

/// The answer's shape: `{"items": [{"id", "text"}]}`.
pub(crate) fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "text": { "type": "string" },
                    },
                    "required": ["id", "text"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["items"],
        "additionalProperties": false,
    })
}

/// A model's answer, read leniently: a model that wraps its JSON in a
/// Markdown fence, or says something before it, still answered.
pub(crate) fn answer(text: &str, who: &str) -> Result<Response, String> {
    let start = text.find('{');
    let end = text.rfind('}');
    let json = match (start, end) {
        (Some(s), Some(e)) if s < e => &text[s..=e],
        _ => text,
    };
    serde_json::from_str(json).map_err(|e| format!("{who}'s answer is not the JSON asked for: {e}"))
}
