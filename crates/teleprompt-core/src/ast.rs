use crate::SourceSpan;

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
}

#[derive(Debug, Clone)]
pub enum Node {
    Segment(Segment),
    ActionBlock(ActionBlock),
    Directive(Directive),
}

#[derive(Debug, Clone)]
pub struct Segment {
    pub id: Option<String>,
    pub text: String,
    pub raw_attrs: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone)]
pub struct ActionBlock {
    pub id: Option<String>,
    pub info: String,
    pub body: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    Pause(u64),
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
