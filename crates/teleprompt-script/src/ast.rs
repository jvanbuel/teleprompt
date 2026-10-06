use teleprompt_core::SourceSpan;
use teleprompt_core::{BlockId, DurationMs, LineId};

#[derive(Debug, Clone)]
pub struct Script {
    pub front_matter: String,
    pub chapters: Vec<Chapter>,
}

#[derive(Debug, Clone)]
pub struct Chapter {
    pub title: String,
    pub slug: String,
    pub nodes: Vec<Node>,
    /// The YAML body of a ` ```yaml teleprompt ` block appearing immediately
    /// after this chapter's heading (config layer 4,
    /// docs/design.md#configuration). Empty when the chapter has no such block.
    pub front_matter: String,
    /// The `{…}` after its title, as in `# Interview {#interview
    /// speaker=guest}`, without the braces; its `#id`, if any, is `slug`.
    pub raw_attrs: String,
    /// Where its heading is.
    pub span: SourceSpan,
}

#[derive(Debug, Clone)]
pub enum Node {
    Line(Line),
    ActionBlock(ActionBlock),
    Directive(Directive),
}

#[derive(Debug, Clone)]
pub struct Line {
    /// Its `{#id}`, or one derived from its chapter: never missing.
    pub id: LineId,
    pub id_origin: IdOrigin,
    /// The bold label it opens with, as in `**Guest:** Hello.`, without
    /// its colon: who says it if the cast has them, else read aloud.
    pub label: Option<String>,
    /// What it says, after any label.
    pub text: String,
    pub raw_attrs: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdOrigin {
    Explicit,
    Derived,
}

#[derive(Debug, Clone)]
pub struct ActionBlock {
    /// Its `id=`, or one derived from the line before it: never missing.
    pub id: BlockId,
    pub info: String,
    pub body: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    Pause(DurationMs),
}

/// A cast member's key as a label or the screen writes their name:
/// `guest` is `Guest`, and `ada-lovelace` is `Ada Lovelace`.
pub fn name_of(key: &str) -> String {
    key.split(['-', '_'])
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}
