//! `teleprompt translate <script> --to <locale>`: the script's narration
//! translated into `<script>.<locale>.yaml`, asking the translator only for
//! what is missing or has changed in the English since.

use std::path::{Path, PathBuf};

use serde::Serialize;
use teleprompt_core::translation::{items, merged, pending, Translation};
use teleprompt_translate::{Known, Request, Translator, Wanted};

use crate::cmd::check::{source_program, translation_path};
use crate::project::Project;

pub enum TranslateError {
    /// The script, or the locale asked for, is wrong.
    Validation(Vec<String>),
    Runtime(String),
}

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

pub async fn run_translate(
    project: &Project,
    script: &Path,
    target: &str,
    translator: &Translator,
) -> Result<TranslateReport, TranslateError> {
    let program = source_program(project, script).map_err(TranslateError::Validation)?;
    let source = program.locale.clone();
    if target == source {
        return Err(TranslateError::Validation(vec![format!(
            "{} is written in `{source}`; translate it --to another locale",
            script.display()
        )]));
    }
    let file = translation_path(script, target);
    let existing = match std::fs::read_to_string(&file) {
        Ok(yaml) => Translation::from_yaml(&yaml)
            .map_err(|e| TranslateError::Validation(vec![format!("{}: {e}", file.display())]))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Translation::default(),
        Err(e) => {
            return Err(TranslateError::Runtime(format!(
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
        .map_err(TranslateError::Runtime)?
        .into_iter()
        .filter(|(id, _)| asked.contains(id))
        .collect();

    let (translation, rejected) = merged(&program, &existing, &done);
    let yaml = translation.to_yaml(&program.script_name, target);
    let partial = file.with_extension("yaml.partial");
    std::fs::write(&partial, yaml)
        .and_then(|()| std::fs::rename(&partial, &file))
        .map_err(|e| TranslateError::Runtime(format!("cannot write {}: {e}", file.display())))?;

    let answered = |id: &String| done.iter().any(|(d, _)| d == id) && !rejected.contains(id);
    Ok(TranslateReport {
        file,
        translated: asked.iter().filter(|id| answered(id)).cloned().collect(),
        missing: asked.iter().filter(|id| !answered(id)).cloned().collect(),
    })
}
