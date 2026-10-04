//! `teleprompt lsp`: the language server, with the real compile as its
//! analyzer. What the protocol needs is in `teleprompt-lsp`; here is what
//! only the CLI knows: the project, its cast and scenes, and what a script
//! compiles to.

use std::path::Path;

use teleprompt_core::config::{Config, PartialConfig};
use teleprompt_core::program::Element;
use teleprompt_core::{Diagnostic, SourceSpan};
use teleprompt_lsp::{Analysis, Analyzer, Definition, Project as Outline};

use crate::project::Compiled;
use crate::project::Project;

/// Serves on stdin and stdout until the editor says to exit.
pub fn run_lsp() -> Result<(), String> {
    teleprompt_lsp::run(ProjectAnalyzer).map_err(|e| e.to_string())
}

pub struct ProjectAnalyzer;

impl Analyzer for ProjectAnalyzer {
    fn project(&self, script: &Path) -> Outline {
        let script_dir = crate::project::script_dir(script).to_path_buf();
        let Ok(project) = Project::for_script(script) else {
            return Outline {
                script_dir,
                ..Outline::default()
            };
        };
        let toml_path = project.config_path();
        let toml = std::fs::read_to_string(&toml_path).unwrap_or_default();
        let script_text = std::fs::read_to_string(script).unwrap_or_default();
        let config = merged(&project, &script_text);
        let speakers = config
            .voices
            .iter()
            .map(|(name, voice)| {
                let backend = voice.backend.as_deref().unwrap_or(&config.voice.backend);
                let detail = match &voice.voice {
                    Some(v) => format!("{backend} · {v}"),
                    None => backend.to_string(),
                };
                let location = line_of(&toml, &format!("[voices.{name}]"))
                    .map(|l| (toml_path.clone(), l))
                    .or_else(|| {
                        line_of(&script_text, &format!("{name}:"))
                            .map(|l| (script.to_path_buf(), l))
                    });
                Definition {
                    name: name.clone(),
                    detail,
                    location,
                }
            })
            .collect();
        let mut scenes: Vec<Definition> = config
            .scenes
            .iter()
            .map(|(name, scene)| Definition {
                name: name.clone(),
                detail: scene.plugin.clone(),
                location: line_of(&toml, &format!("[scene.{name}]"))
                    .map(|l| (toml_path.clone(), l)),
            })
            .collect();
        for plugin in crate::scene::plugins().names() {
            if !scenes.iter().any(|s| s.name == plugin) {
                scenes.push(Definition {
                    name: plugin.to_string(),
                    detail: "scene plugin".into(),
                    location: None,
                });
            }
        }
        Outline {
            speakers,
            scenes,
            script_dir,
        }
    }

    fn analyze(&self, script: &Path, text: &str) -> Analysis {
        let project = match Project::for_script(script) {
            Ok(project) => project,
            Err(e) => {
                return Analysis {
                    diagnostics: vec![Diagnostic::error(e.to_string())],
                    ..Analysis::default()
                }
            }
        };
        let locale = project.source_locale();
        match project.compile_source(&project.backends(), script, text, &locale) {
            Ok(compiled) => analysis(&compiled),
            Err(d) => Analysis {
                diagnostics: d.0,
                ..Analysis::default()
            },
        }
    }
}

/// The project's configuration with `script_text`'s front matter over it.
fn merged(project: &Project, script_text: &str) -> Config {
    let front = teleprompt_core::parse::parse_script(script_text)
        .ok()
        .and_then(|s| PartialConfig::from_yaml(&s.front_matter).ok())
        .unwrap_or_default();
    Config::merged(&[project.config.clone(), front])
}

/// The line, from 0, of the first line of `text` that reads `needle`.
fn line_of(text: &str, needle: &str) -> Option<u32> {
    text.lines()
        .position(|l| l.trim() == needle)
        .map(|n| n as u32)
}

/// A compiled script's warnings, where they are, and what each line and
/// block compiles to.
fn analysis(compiled: &Compiled) -> Analysis {
    let mut out = Analysis {
        diagnostics: teleprompt_core::lint::lint(&compiled.program),
        ..Analysis::default()
    };
    // The compile's warnings are sentences; most name their line.
    let spans: Vec<(String, SourceSpan)> = compiled
        .program
        .elements
        .iter()
        .filter_map(|e| match e {
            Element::Narration { id, span, .. } => Some((id.to_string(), *span)),
            _ => None,
        })
        .collect();
    for warning in &compiled.output.warnings {
        let span = spans
            .iter()
            .find(|(id, _)| warning.contains(&format!("`{id}`")))
            .map(|(_, s)| *s);
        let d = Diagnostic::warning(warning.clone());
        out.diagnostics.push(match span {
            Some(span) => d.at(span),
            None => d,
        });
    }
    out.hovers = hovers(compiled);
    out
}

/// What each line and block compiles to, as Markdown, by the source line
/// it starts on.
fn hovers(compiled: &Compiled) -> std::collections::BTreeMap<usize, String> {
    let timeline = &compiled.output.timeline;
    let mut out = std::collections::BTreeMap::new();
    for element in &compiled.program.elements {
        match element {
            Element::Narration {
                id, speaker, span, ..
            } => {
                let Some(detail) = compiled.output.narration.iter().find(|d| &d.line_id == id)
                else {
                    continue;
                };
                let Some(n) = timeline
                    .entries
                    .iter()
                    .filter_map(|e| e.narration.as_ref())
                    .find(|n| &n.line == id)
                else {
                    continue;
                };
                let who = speaker
                    .as_ref()
                    .map_or(String::new(), |s| format!(" · said by {s}"));
                let voice = match (&detail.take, &detail.synth_request.voice) {
                    (Some(_), _) => "your take".to_string(),
                    (None, Some(v)) => format!("{} · {v}", detail.backend),
                    (None, None) => detail.backend.clone(),
                };
                out.insert(
                    span.line,
                    format!(
                        "**line `{id}`**{who}\n\n{voice}, {} ({}) at {}",
                        seconds(n.duration_ms.ms()),
                        n.duration_source,
                        clock(n.start_ms.ms())
                    ),
                );
            }
            Element::Action {
                block_id,
                scene,
                policy,
                span,
                ..
            } => {
                let shots: Vec<_> = timeline
                    .entries
                    .iter()
                    .filter_map(|e| e.action.as_ref())
                    .filter(|a| a.shot.to_string().starts_with(&format!("{block_id}#")))
                    .collect();
                let Some(first) = shots.first() else {
                    continue;
                };
                let total: u64 = shots.iter().map(|a| a.duration_ms.ms()).sum();
                let noun = if shots.len() == 1 { "shot" } else { "shots" };
                out.insert(
                    span.line,
                    format!(
                        "**block `{block_id}`** · scene `{scene}` ({}) · {policy}\n\n{} {noun}, {} at {}",
                        first.plugin,
                        shots.len(),
                        seconds(total),
                        clock(first.start_ms.ms())
                    ),
                );
            }
            Element::Pause { .. } => {}
        }
    }
    out
}

fn seconds(ms: u64) -> String {
    format!("{:.1} s", ms as f64 / 1000.0)
}

/// `m:ss.s`, where it plays in the video.
fn clock(ms: u64) -> String {
    format!("{}:{:04.1}", ms / 60_000, (ms % 60_000) as f64 / 1000.0)
}
