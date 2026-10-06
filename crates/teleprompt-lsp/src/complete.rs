//! What completion offers where the cursor is: after `**` opening a
//! paragraph, the cast; in a line's closing braces, its attributes; in a
//! heading's, a chapter's settings; on a ` ```teleprompt ` fence, a
//! block's attributes; after `key=`, that key's values.

use lsp_types::{CompletionItem, CompletionItemKind, Position};
use teleprompt_core::attrs::{BLOCK_KEYS, SEGMENT_KEYS};

use crate::text::LineIndex;
use crate::Project;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Context {
    /// A key among a line's attributes.
    LineKey,
    /// A speaker's label, after `**` opening a paragraph.
    Speaker,
    /// A setting in a chapter heading's braces.
    HeadingKey,
    /// A key on a block's fence.
    BlockKey,
    /// The value of `key`, after `key=`.
    Value { key: String },
}

/// What is being typed at `position` in `text`, and what of it is typed
/// so far; `None` where nothing is completed.
pub fn context(text: &str, position: Position) -> Option<(Context, String)> {
    let index = LineIndex::new(text);
    let line = index.line(position.line as usize);
    let column = index.offset(position) - line_start(&index, position);
    let before = &line[..column.min(line.len())];
    if let Some(name) = label_typed(&index, position.line as usize, before) {
        return Some((Context::Speaker, name.into()));
    }
    let heading = line.starts_with('#');
    let fence = line.trim_start().strip_prefix("```teleprompt");
    let attrs = match fence {
        Some(rest) => {
            let from = line.len() - rest.len();
            if before.len() < from || (!rest.is_empty() && !rest.starts_with(' ')) {
                return None;
            }
            &before[from..]
        }
        None => {
            let open = before.rfind('{')?;
            if before[open..].contains('}') {
                return None;
            }
            &before[open + 1..]
        }
    };
    let token = attrs.rsplit(char::is_whitespace).next().unwrap_or("");
    let ctx = if let Some((key, value)) = token.split_once('=') {
        return Some((
            Context::Value { key: key.into() },
            value.trim_start_matches('"').into(),
        ));
    } else if fence.is_some() {
        Context::BlockKey
    } else if token.starts_with('#') {
        return None;
    } else if heading {
        Context::HeadingKey
    } else {
        Context::LineKey
    };
    Some((ctx, token.into()))
}

/// The name typed so far after `**` or `__` opening a paragraph.
fn label_typed<'a>(index: &LineIndex, line: usize, before: &'a str) -> Option<&'a str> {
    let opens = line == 0 || {
        let above = index.line(line - 1).trim();
        above.is_empty() || above.starts_with('#')
    };
    let name = before
        .strip_prefix("**")
        .or_else(|| before.strip_prefix("__"))?;
    (opens && !name.contains([':', '*'])).then_some(name)
}

/// Settings a chapter's heading takes most, of all front matter does.
const HEADING_KEYS: &[&str] = &[
    "speaker",
    "voice.backend",
    "voice.voice",
    "voice.speed",
    "voice.instruct",
    "timing.lead_in_ms",
    "timing.tail_ms",
];

fn line_start(index: &LineIndex, position: Position) -> usize {
    index.offset(Position {
        line: position.line,
        character: 0,
    })
}

/// What policy= and align= may be.
const POLICIES: &[&str] = &[
    "hold",
    "concurrent",
    "fit-action",
    "trim-action",
    "fit-line",
];
const ALIGNS: &[&str] = &["start", "end", "center"];

/// Everything `ctx` may be, before the editor filters by what is typed.
pub fn items(ctx: &Context, project: &Project) -> Vec<CompletionItem> {
    let item = |label: String, detail: &str, kind, insert: String| CompletionItem {
        label,
        detail: (!detail.is_empty()).then(|| detail.to_string()),
        kind: Some(kind),
        insert_text: Some(insert),
        ..CompletionItem::default()
    };
    let keys = |keys: &[&str]| {
        keys.iter()
            .map(|k| {
                item(
                    (*k).into(),
                    "",
                    CompletionItemKind::PROPERTY,
                    format!("{k}="),
                )
            })
            .collect::<Vec<_>>()
    };
    let named = |defs: &[crate::Definition], kind| {
        defs.iter()
            .map(|d| item(d.name.clone(), &d.detail, kind, d.name.clone()))
            .collect::<Vec<_>>()
    };
    let values = |values: &[&str]| {
        values
            .iter()
            .map(|v| {
                item(
                    (*v).into(),
                    "",
                    CompletionItemKind::ENUM_MEMBER,
                    (*v).into(),
                )
            })
            .collect()
    };
    match ctx {
        Context::Speaker => project
            .speakers
            .iter()
            .map(|d| {
                let label = capitalized(&d.name);
                let insert = format!("{label}:** ");
                item(label, &d.detail, CompletionItemKind::VARIABLE, insert)
            })
            .collect(),
        Context::LineKey => keys(SEGMENT_KEYS),
        Context::HeadingKey => keys(HEADING_KEYS),
        Context::BlockKey => keys(BLOCK_KEYS),
        Context::Value { key } => match key.as_str() {
            "scene" => named(&project.scenes, CompletionItemKind::MODULE),
            "speaker" => named(&project.speakers, CompletionItemKind::VARIABLE),
            "policy" => values(POLICIES),
            "align" => values(ALIGNS),
            "include" => files(project),
            _ => Vec::new(),
        },
    }
}

/// A cast member's name as a label writes it: `guest` is `Guest`.
pub fn capitalized(name: &str) -> String {
    teleprompt_script::ast::name_of(name)
}

/// The files beside the script, for `include=`.
fn files(project: &Project) -> Vec<CompletionItem> {
    let Ok(entries) = std::fs::read_dir(&project.script_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let dir = e.file_type().is_ok_and(|t| t.is_dir());
            (!name.starts_with('.')).then(|| if dir { format!("{name}/") } else { name })
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| CompletionItem {
            kind: Some(if name.ends_with('/') {
                CompletionItemKind::FOLDER
            } else {
                CompletionItemKind::FILE
            }),
            insert_text: Some(name.clone()),
            label: name,
            ..CompletionItem::default()
        })
        .collect()
}
