//! Translating a script's narration: what `teleprompt translate` sends a
//! translator, and the translators it can send it to.
//!
//! Every translator gets the same [`Request`], the items to translate and
//! the translations already made, and answers with a [`Response`], one
//! text per item it translated. A translator that answers for only some
//! items leaves the rest for next time.
//!
//! A provider is chosen by name ([`PROVIDERS`]), with its own settings from
//! `backends.<name>`. Adding one is a module with a `translate` method and
//! an arm in [`Translator::new`]; `command` and `openai` reach anything
//! else without a new build.

use serde::{Deserialize, Serialize};
use teleprompt_core::translation::{Item, Kind};

mod claude;
mod command;
pub mod ollama;
pub mod openai;
mod prompt;

pub use claude::Claude;
pub use ollama::Ollama;
pub use openai::OpenAi;

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

/// The providers this build knows, the default first.
pub const PROVIDERS: &[&str] = &["ollama", "openai", "claude", "command"];

/// Something that translates.
pub enum Translator {
    /// A model run locally by Ollama.
    Ollama(Ollama),
    /// A server speaking the OpenAI chat completions API.
    OpenAi(OpenAi),
    /// Claude, through the Anthropic API.
    Claude(Claude),
    /// A program of the author's, run by the shell, given the request on
    /// stdin and answering on stdout.
    Command(String),
}

/// `[backends.command]`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandSettings {
    run: Option<String>,
}

impl Translator {
    /// The provider named `provider`, with `model` if given, configured
    /// from its `backends.<provider>` section.
    pub fn new(
        provider: &str,
        model: Option<&str>,
        settings: Option<&serde_yaml::Value>,
    ) -> Result<Self, String> {
        fn read<T: serde::de::DeserializeOwned + Default>(
            provider: &str,
            settings: Option<&serde_yaml::Value>,
        ) -> Result<T, String> {
            settings.map_or_else(
                || Ok(T::default()),
                |v| {
                    serde_yaml::from_value(v.clone())
                        .map_err(|e| format!("backends.{provider}: {e}"))
                },
            )
        }
        match provider {
            "ollama" => Ok(Translator::Ollama(Ollama::new(
                model,
                read(provider, settings)?,
            ))),
            "openai" => Ok(Translator::OpenAi(OpenAi::new(
                model,
                read(provider, settings)?,
            )?)),
            "claude" => Ok(Translator::Claude(Claude::from_env(model)?)),
            "command" => {
                let s: CommandSettings = read(provider, settings)?;
                s.run.map(Translator::Command).ok_or_else(|| {
                    "the command provider needs a program: set `run` under [backends.command], \
                     or pass --command"
                        .to_string()
                })
            }
            other => Err(format!(
                "unknown translation provider `{other}`; known: {}",
                PROVIDERS.join(", ")
            )),
        }
    }

    /// Items asked for in one request: fewer for a model on a laptop, whose
    /// answers are slow and whose context is short.
    fn batch(&self) -> usize {
        match self {
            Translator::Ollama(_) | Translator::OpenAi(_) => 20,
            Translator::Claude(_) | Translator::Command(_) => 60,
        }
    }

    /// Translates `request`, a batch at a time, returning each text by id.
    pub async fn translate(&self, request: &Request) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        for batch in request.translate.chunks(self.batch()) {
            let part = Request {
                translate: batch.to_vec(),
                ..request.clone()
            };
            let response = match self {
                Translator::Ollama(o) => o.translate(&part).await?,
                Translator::OpenAi(o) => o.translate(&part).await?,
                Translator::Claude(c) => c.translate(&part).await?,
                Translator::Command(cmd) => command::translate(cmd, &part).await?,
            };
            out.extend(response.items.into_iter().map(|t| (t.id, t.text)));
        }
        Ok(out)
    }
}
