//! Lines that are hard to say aloud, found before anything is synthesized:
//! `check`'s warnings about the narration itself.

use crate::program::{Element, Program};
use crate::Diagnostic;

/// Words in one sentence past which it is hard to say in a breath, and to
/// read as captions.
const LONGEST_SENTENCE: usize = 30;

/// Words that repeat in correct English.
const REPEATABLE: &[&str] = &["that", "had"];

pub fn lint(program: &Program) -> Vec<Diagnostic> {
    let mut out = program.warnings.clone();
    for element in &program.elements {
        let Element::Narration {
            id,
            text,
            config,
            span,
            ..
        } = element
        else {
            continue;
        };
        for sentence in sentences(text) {
            let words = sentence.split_whitespace().count();
            if words > LONGEST_SENTENCE {
                out.push(
                    Diagnostic::warning(format!(
                        "line `{id}` has a {words}-word sentence, hard to say in one breath \
                         and to read as captions"
                    ))
                    .at(*span)
                    .with_help("split it in two"),
                );
            }
        }
        for word in text.split_whitespace() {
            let word = bare(word);
            if looks_like_code(word) && !config.voice.pronounce.contains_key(word.trim_matches('`'))
            {
                let shown = word.trim_matches('`');
                out.push(
                    Diagnostic::warning(format!(
                        "line `{id}` says `{shown}`, which a voice may read out character by \
                         character"
                    ))
                    .at(*span)
                    .with_help(format!(
                        "reword it, or say how under voice.pronounce, e.g. \
                         pronounce: {{ \"{shown}\": \"…\" }}"
                    )),
                );
            }
        }
        let tokens: Vec<&str> = text.split_whitespace().collect();
        for pair in tokens.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let same = !a.ends_with(|c: char| c.is_ascii_punctuation())
                && bare(a).eq_ignore_ascii_case(bare(b))
                && !REPEATABLE.contains(&bare(a).to_lowercase().as_str());
            if same && bare(a).chars().any(char::is_alphabetic) {
                out.push(
                    Diagnostic::warning(format!("line `{id}` says \"{a} {b}\""))
                        .at(*span)
                        .with_help("probably a typo: remove one"),
                );
            }
        }
    }
    out
}

/// The sentences of a line, ended by `.`, `!` or `?` before a space.
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        let ends = matches!(c, '.' | '!' | '?') && bytes.get(i + 1).is_none_or(|b| *b == b' ');
        if ends {
            out.push(&text[start..=i]);
            start = i + 1;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// A word without the punctuation around it in the sentence.
fn bare(word: &str) -> &str {
    word.trim_start_matches(['(', '"', '\'', '“', '‘'])
        .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '"', '\'', '”', '’'])
}

/// What a voice would spell out: a URL, a path, a file name, a flag, an
/// identifier, or anything in backticks.
fn looks_like_code(word: &str) -> bool {
    if word.contains('`') || word.contains("://") || word.contains("::") || word.contains('=') {
        return true;
    }
    if word.starts_with("--") && word.len() > 2 {
        return true;
    }
    if word.contains('/') && word != "and/or" {
        let dotted = word.split('/').any(|part| part.contains('.'));
        return dotted || word.starts_with(['/', '~']) || word.matches('/').count() > 1;
    }
    let letters_around = |sep: char| {
        word.split(sep).count() > 1
            && word
                .split(sep)
                .all(|part| part.chars().next().is_some_and(char::is_alphanumeric))
    };
    if letters_around('_') {
        return true;
    }
    file_name(word) || camel_case(word)
}

/// `tour.md`, `README.md`: a stem and a short lowercase extension. Not
/// `e.g` or `3.5`.
fn file_name(word: &str) -> bool {
    let Some((stem, ext)) = word.rsplit_once('.') else {
        return false;
    };
    stem.chars().count() >= 2
        && stem.chars().any(char::is_alphabetic)
        && (1..=4).contains(&ext.len())
        && ext
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && ext.chars().any(|c| c.is_ascii_lowercase())
}

/// `maxStretch`: lowercase letters, then a capital, then lowercase. Not
/// `iPhone` or `macOS`.
fn camel_case(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    chars.first().is_some_and(|c| c.is_lowercase())
        && chars.windows(4).any(|w| {
            w[0].is_lowercase() && w[1].is_lowercase() && w[2].is_uppercase() && w[3].is_lowercase()
        })
}
