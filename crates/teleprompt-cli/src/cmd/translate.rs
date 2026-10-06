//! `teleprompt translate <script> --to <locale>`: the script's narration
//! translated into `<script>.<locale>.yaml`, asking the translator only for
//! what is missing or has changed in the English since.

use std::path::PathBuf;

use serde::Serialize;
use teleprompt_core::translation::{items, merged, pending, Translation};
use teleprompt_translate::{Known, Request, Translator, Wanted};

use crate::output::Failure;
use crate::project::translation_path;
use crate::project::Script;

#[derive(Debug, Serialize)]
pub struct TranslateReport {
    pub file: PathBuf,
    /// Ids translated this time (`line:welcome`, `cue:start-a`).
    pub translated: Vec<String>,
    /// Asked for, and not answered or not usable: still to translate.
    pub missing: Vec<String>,
}

impl TranslateReport {
    pub fn render(&self) -> String {
        if self.translated.is_empty() && self.missing.is_empty() {
            return format!("{} is up to date\n", self.file.display());
        }
        let mut s = format!(
            "translated {} item(s) into {}\n",
            self.translated.len(),
            self.file.display()
        );
        if !self.missing.is_empty() {
            s.push_str(&format!(
                "  still to translate: {}\n  run translate again, or write them in the file\n",
                self.missing.join(", ")
            ));
        }
        s.push_str("  read it over: it is spoken exactly as written\n");
        s
    }
}

/// What to translate with, overriding `[translate]` for one run.
#[derive(Default)]
pub struct Choice<'a> {
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    /// A shell command to translate with: the `command` provider.
    pub command: Option<&'a str>,
}

/// Translating: the script in the locale it is translated into.
impl Script {
    /// The translator `[translate]` names for this locale, as `choice`
    /// overrides it.
    pub fn translator(&self, choice: &Choice) -> Result<Translator, Failure> {
        let (project, script, target) = (self.project(), self.path(), self.locale());
        let config = project
            .resolved(script, target)
            .map_err(Failure::Validation)?
            .config;
        let timeout_ms = config.translate.timeout_ms;
        if let Some(cmd) = choice.command {
            let program = teleprompt_translate::Program::new(cmd);
            return Ok(Translator::Command(program).timeout(timeout_ms));
        }
        let provider = choice.provider.unwrap_or(&config.translate.provider);
        // A model named for one provider means nothing to another.
        let model = choice.model.or(if choice.provider.is_none() {
            config.translate.model.as_deref()
        } else {
            None
        });
        Translator::new(provider, model, config.translate.settings.get(provider))
            .map(|t| t.timeout(timeout_ms))
            .map_err(Failure::Runtime)
    }

    /// Translates what is missing from, or has changed since, this
    /// locale's translation, and writes it beside the script.
    pub async fn translate(&self, translator: &Translator) -> Result<TranslateReport, Failure> {
        let (project, script, target) = (self.project(), self.path(), self.locale());
        let program = project
            .source_program(script)
            .map_err(Failure::Validation)?;
        let source = program.locale.clone();
        if target == source {
            return Err(Failure::Validation(vec![format!(
                "{} is written in `{source}`; translate it --to another locale",
                script.display()
            )]));
        }
        let file = translation_path(script, target);
        let existing = match std::fs::read_to_string(&file) {
            Ok(yaml) => Translation::from_yaml(&yaml)
                .map_err(|e| Failure::Validation(vec![format!("{}: {e}", file.display())]))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Translation::default(),
            Err(e) => {
                return Err(Failure::Runtime(format!(
                    "cannot read {}: {e}",
                    file.display()
                )))
            }
        };

        let todo = pending(&program, &existing);
        if todo.is_empty() {
            return Ok(TranslateReport {
                file,
                translated: Vec::new(),
                missing: Vec::new(),
            });
        }
        let asked: Vec<String> = todo.iter().map(|i| i.id()).collect();
        let request = Request {
            source,
            target: target.to_string(),
            existing: items(&program)
                .iter()
                .filter(|i| !asked.contains(&i.id()))
                .filter_map(|i| {
                    existing.of(i).map(|e| Known {
                        id: i.id(),
                        english: i.english.clone(),
                        translation: e.text.clone(),
                    })
                })
                .collect(),
            translate: todo.iter().map(Wanted::from).collect(),
        };
        let done: Vec<(String, String)> = translator
            .translate(&request)
            .await
            .map_err(Failure::Runtime)?
            .into_iter()
            .filter(|(id, _)| asked.contains(id))
            .collect();

        let (translation, rejected) = merged(&program, &existing, &done);
        let yaml = translation.to_yaml(&program.script_name, target);
        let partial = file.with_extension("yaml.partial");
        std::fs::write(&partial, yaml)
            .and_then(|()| std::fs::rename(&partial, &file))
            .map_err(|e| Failure::Runtime(format!("cannot write {}: {e}", file.display())))?;

        let answered = |id: &String| done.iter().any(|(d, _)| d == id) && !rejected.contains(id);
        Ok(TranslateReport {
            file,
            translated: asked.iter().filter(|id| answered(id)).cloned().collect(),
            missing: asked.iter().filter(|id| !answered(id)).cloned().collect(),
        })
    }
}

/// `translate`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    pub script: std::path::PathBuf,
    /// The locale to translate into, e.g. nl, fr, pt-BR
    #[arg(long, value_parser = crate::cli::language_tag)]
    pub to: String,
    /// Translate with this provider: ollama, openai, claude or command
    #[arg(long, conflicts_with = "command")]
    pub provider: Option<String>,
    /// The provider's model
    #[arg(long, conflicts_with = "command")]
    pub model: Option<String>,
    /// Translate with this shell command: it gets the request as JSON on
    /// stdin and answers {"items": [{"id", "text"}]} on stdout
    #[arg(long)]
    pub command: Option<String>,
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    use crate::output::Outcome;
    let choice = Choice {
        provider: args.provider.as_deref(),
        model: args.model.as_deref(),
        command: args.command.as_deref(),
    };
    let project = crate::cli::project_for(&args.script)?;
    let script = project.script(&args.script, &args.to);
    let translator = script.translator(&choice)?;
    let report = crate::cli::runtime()?.block_on(script.translate(&translator))?;
    crate::cli::emit(format, &report, &report.render());
    Ok(Outcome::Ok)
}
