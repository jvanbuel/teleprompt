//! Translating a script's narration: what `teleprompt translate` sends a
//! translator, and the translators it can send it to.
//!
//! Every translator gets the same [`Request`], the items to translate and
//! the translations already made, and answers with a [`Response`], one
//! text per item it translated. A translator that answers for only some
//! items leaves the rest for next time.

use serde::{Deserialize, Serialize};
use teleprompt_core::translation::{Item, Kind};

mod claude;
mod command;

pub use claude::Claude;

/// What a translator is asked, as JSON.
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    /// Locale codes, as the script uses them (`en`, `nl`, `pt-BR`).
    pub source: String,
    pub target: String,
    /// Translations already made, for the terminology to stay the same.
    pub existing: Vec<Known>,
    pub translate: Vec<Wanted>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Known {
    pub id: String,
    pub english: String,
    pub translation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Wanted {
    pub id: String,
    /// `chapter`, `line` or `cue`.
    pub kind: &'static str,
    pub english: String,
    /// For a cue: the id of the line it is a phrase of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
}

impl From<&Item> for Wanted {
    fn from(item: &Item) -> Self {
        let (kind, line) = match &item.kind {
            Kind::Chapter => ("chapter", None),
            Kind::Line => ("line", None),
            Kind::Cue { line } => ("cue", Some(format!("line:{line}"))),
        };
        Wanted {
            id: item.id(),
            kind,
            english: item.english.clone(),
            line,
        }
    }
}

/// What a translator answers, as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub items: Vec<Translated>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Translated {
    pub id: String,
    pub text: String,
}

/// Something that translates.
pub enum Translator {
    /// Claude, through the Anthropic API.
    Claude(Claude),
    /// A program of the author's, run by the shell, given the request on
    /// stdin and answering on stdout.
    Command(String),
}

/// Items asked for in one request: enough for a whole script, few enough
/// to keep one answer short.
const BATCH: usize = 60;

impl Translator {
    /// Translates `request`, a batch at a time, returning each text by id.
    pub async fn translate(&self, request: &Request) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        for batch in request.translate.chunks(BATCH) {
            let part = Request {
                translate: batch.to_vec(),
                ..request.clone()
            };
            let response = match self {
                Translator::Claude(c) => c.translate(&part).await?,
                Translator::Command(cmd) => command::translate(cmd, &part).await?,
            };
            out.extend(response.items.into_iter().map(|t| (t.id, t.text)));
        }
        Ok(out)
    }
}
