//! A script's narration in another language: a sidecar beside the script
//! (`scripts/tour.nl.yaml` for `scripts/tour.md`), applied to its program
//! when it is compiled for that locale.
//!
//! Every entry records which English it was translated from, as the start
//! of that text's hash, so an entry whose English has since changed is
//! reported as out of date rather than silently spoken.

use crate::program::{Element, Program};
use crate::{Diagnostic, Hash};

/// A translated chapter title, line or cue, keyed by chapter slug, line id
/// or block id. Kept in the order written, which is the script's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Translation {
    pub chapters: Vec<(String, Entry)>,
    pub lines: Vec<(String, Entry)>,
    pub cues: Vec<(String, Entry)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// [`source_of`] the English this was translated from.
    pub from: String,
    pub text: String,
}

/// Hex characters of a source's hash an entry keeps: enough to tell two
/// versions of a line apart, short enough to read past in a review.
const SOURCE_CHARS: usize = 12;

/// What an entry records of the English it was translated from.
pub fn source_of(english: &str) -> String {
    Hash::of(english.as_bytes()).to_string()[..SOURCE_CHARS].to_string()
}

fn find<'a>(entries: &'a [(String, Entry)], key: &str) -> Option<&'a Entry> {
    entries.iter().find(|(k, _)| k == key).map(|(_, e)| e)
}

/// Puts `translation` in place of `program`'s English: its lines, chapter
/// titles and cues. A line with no translation is an error; one translated
/// from English that has since changed is a warning.
pub fn apply(program: &mut Program, translation: &Translation) -> Vec<Diagnostic> {
    let locale = program.locale.clone();
    let mut diags = Vec::new();
    for chapter in &mut program.chapters {
        if let Some(e) = find(&translation.chapters, &chapter.slug) {
            if e.from != source_of(&chapter.title) {
                diags.push(stale("chapter title", &chapter.slug, &locale));
            }
            chapter.title = e.text.clone();
        }
    }
    // The line above each block, in English and translated, for its cue.
    let mut above: Option<(String, String)> = None;
    for element in &mut program.elements {
        match element {
            Element::Narration {
                id,
                text,
                source_hash,
                span,
                ..
            } => {
                let Some(e) = find(&translation.lines, id) else {
                    diags.push(
                        Diagnostic::error(format!("line `{id}` has no `{locale}` translation"))
                            .at(*span)
                            .with_help(format!(
                                "run `teleprompt translate <script> --to {locale}`, or add it \
                                 to the script's .{locale}.yaml"
                            )),
                    );
                    above = None;
                    continue;
                };
                if e.from != source_of(text) {
                    diags.push(stale("line", id, &locale).at(*span));
                }
                above = Some((std::mem::replace(text, e.text.clone()), e.text.clone()));
                *source_hash = Hash::of(text.as_bytes());
            }
            Element::Action {
                block_id,
                cue: Some(cue),
                span,
                ..
            } => {
                if let Some(e) = find(&translation.cues, block_id) {
                    if e.from != source_of(cue) {
                        diags.push(stale("cue", block_id, &locale).at(*span));
                    }
                    *cue = e.text.clone();
                } else if let Some((english, translated)) = &above {
                    if !translated.contains(cue.as_str()) {
                        if let Some(guess) = same_place(english, translated, cue) {
                            diags.push(
                                Diagnostic::warning(format!(
                                    "`cue=\"{cue}\"` is not in the `{locale}` line, so the shot \
                                     starts on \"{guess}\", as far into it"
                                ))
                                .at(*span)
                                .with_help(format!(
                                    "give block `{block_id}` its cue under `cues:` in the \
                                     script's .{locale}.yaml"
                                )),
                            );
                            *cue = guess;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    diags
}

fn stale(what: &str, key: &str, locale: &str) -> Diagnostic {
    Diagnostic::warning(format!(
        "the English of {what} `{key}` has changed since its `{locale}` translation"
    ))
    .with_help(format!(
        "run `teleprompt translate <script> --to {locale}` to translate it again"
    ))
}

/// The words of `translated` as far into it as `cue` is into `english`,
/// grown until they occur in it once.
fn same_place(english: &str, translated: &str, cue: &str) -> Option<String> {
    let at = english.find(cue)?;
    let share = english[..at].chars().count() as f64 / english.chars().count().max(1) as f64;
    let words: Vec<(usize, &str)> = translated
        .split(' ')
        .scan(0, |offset, w| {
            let start = *offset;
            *offset += w.chars().count() + 1;
            Some((start, w))
        })
        .filter(|(_, w)| !w.is_empty())
        .collect();
    let target = share * translated.chars().count() as f64;
    let first = (0..words.len()).min_by(|&a, &b| {
        let d = |i: usize| (words[i].0 as f64 - target).abs();
        d(a).total_cmp(&d(b))
    })?;
    (first + 1..=words.len())
        .map(|end| {
            let phrase: Vec<&str> = words[first..end].iter().map(|(_, w)| *w).collect();
            phrase.join(" ")
        })
        .find(|p| translated.matches(p.as_str()).count() == 1)
}

impl Translation {
    /// The sidecar as YAML, sections and entries in order, under a header
    /// saying what the file is.
    pub fn to_yaml(&self, script: &str, locale: &str) -> String {
        let mut out = format!(
            "# The `{locale}` narration of {script}, kept by `teleprompt translate`.\n\
             # Edit any `text`. `from` is the English it was translated from: when\n\
             # that changes, the entry is reported until it is translated again.\n"
        );
        for (name, entries) in [
            ("chapters", &self.chapters),
            ("lines", &self.lines),
            ("cues", &self.cues),
        ] {
            if entries.is_empty() {
                continue;
            }
            out.push_str(&format!("{name}:\n"));
            for (key, e) in entries {
                out.push_str(&format!(
                    "  {}:\n    from: {}\n    text: {}\n",
                    scalar(key),
                    e.from,
                    scalar(&e.text)
                ));
            }
        }
        out
    }

    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let doc: serde_yaml::Value = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
        let mut t = Translation::default();
        if doc.is_null() {
            return Ok(t);
        }
        let map = doc
            .as_mapping()
            .ok_or("expected `chapters:`, `lines:` and `cues:`")?;
        for (section, value) in map {
            let name = section.as_str().unwrap_or_default();
            let entries = match name {
                "chapters" => &mut t.chapters,
                "lines" => &mut t.lines,
                "cues" => &mut t.cues,
                other => return Err(format!("unknown section `{other}`")),
            };
            let Some(items) = value.as_mapping() else {
                continue;
            };
            for (key, v) in items {
                let key = key
                    .as_str()
                    .ok_or(format!("a key under `{name}:` is not text"))?;
                let field = |f: &str| {
                    v.get(f)
                        .and_then(serde_yaml::Value::as_str)
                        .map(str::to_string)
                        .ok_or(format!("`{name}.{key}` has no `{f}`"))
                };
                entries.push((
                    key.to_string(),
                    Entry {
                        from: field("from")?,
                        text: field("text")?,
                    },
                ));
            }
        }
        Ok(t)
    }
}

/// `s` as a YAML scalar on one line, quoted only where YAML needs it.
fn scalar(s: &str) -> String {
    serde_yaml::to_string(s)
        .map(|y| y.trim_end().to_string())
        .unwrap_or_else(|_| format!("{s:?}"))
}
