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
pub use command::Program;
pub use ollama::Ollama;
pub use openai::OpenAi;

/// How long one batch may take unless `[translate]`'s `timeout_ms` says
/// otherwise.
pub const TIMEOUT_MS: u64 = teleprompt_core::config::TRANSLATE_TIMEOUT_MS;

/// An HTTP client that gives up: on connecting after ten seconds, on the
/// whole request after `timeout_ms`.
fn client(timeout_ms: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
        .unwrap_or_default()
}

/// What to say when `who` has not answered within `timeout_ms`.
fn unanswered(who: &str, timeout_ms: u64) -> String {
    format!("{who} did not answer within {timeout_ms} ms: raise `timeout_ms` under [translate]")
}

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
    Command(Program),
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
            if settings.is_some_and(|v| v.get("timeout_ms").is_some()) {
                return Err(format!(
                    "backends.{provider}: `timeout_ms` is set under [translate] now, \
                     one limit for every provider"
                ));
            }
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
                let program = s.run.as_deref().map(Program::new);
                program.map(Translator::Command).ok_or_else(|| {
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

    /// Gives up on a batch after `timeout_ms`, whichever the provider.
    pub fn timeout(mut self, timeout_ms: u64) -> Self {
        match &mut self {
            Translator::Ollama(o) => o.timeout_ms = timeout_ms,
            Translator::OpenAi(o) => o.timeout_ms = timeout_ms,
            Translator::Claude(c) => c.timeout_ms = timeout_ms,
            Translator::Command(p) => p.timeout_ms = timeout_ms,
        }
        self
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
