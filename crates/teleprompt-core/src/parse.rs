use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::ast::{slugify, ActionBlock, Chapter, Directive, Node, Script, Segment};
use crate::{Diagnostic, Diagnostics, SourceSpan};

const FENCE_TAG: &str = "teleprompt";

pub fn parse_script(src: &str) -> Result<Script, Diagnostics> {
    let (front_matter, body, body_offset) = split_front_matter(src);
    let mut diags = Vec::new();
    let chapters = parse_body(body, body_offset, &mut diags);

    let d = Diagnostics(diags);
    if d.has_errors() {
        return Err(d);
    }
    Ok(Script {
        front_matter,
        chapters,
    })
}

fn split_front_matter(src: &str) -> (String, &str, usize) {
    let Some(rest) = src.strip_prefix("---\n") else {
        return (String::new(), src, 0);
    };
    match rest.find("\n---\n") {
        Some(end) => {
            let fm = rest[..end].to_string();
            let after = &rest[end + 5..];
            let lines = 1 + rest[..end + 5].lines().count();
            (fm, after, lines)
        }
        None => (String::new(), src, 0),
    }
}

fn parse_body(body: &str, line_offset: usize, diags: &mut Vec<Diagnostic>) -> Vec<Chapter> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(body, opts).into_offset_iter();

    let mut chapters: Vec<Chapter> = Vec::new();
    let mut state = State::Idle;
    let mut text = String::new();
    let mut fence_info = String::new();

    for (event, range) in parser {
        let line = line_offset + body[..range.start].lines().count() + 1;
        let span = SourceSpan {
            line,
            column: 1,
            len: range.len(),
        };

        match event {
            Event::Start(Tag::Heading { .. }) => {
                state = State::Heading;
                text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let title = text.trim().to_string();
                chapters.push(Chapter {
                    slug: slugify(&title),
                    title,
                    nodes: Vec::new(),
                });
                state = State::Idle;
                text.clear();
            }
            Event::Start(Tag::Paragraph) => {
                state = State::Paragraph;
                text.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                let raw = text.trim().to_string();
                if !raw.is_empty() {
                    push_node(&mut chapters, paragraph_node(&raw, span), span, diags);
                }
                state = State::Idle;
                text.clear();
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                fence_info = info.to_string();
                state = if fence_info.split_whitespace().next() == Some(FENCE_TAG) {
                    State::ActionBlock
                } else {
                    State::Idle
                };
                text.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                if matches!(state, State::ActionBlock) {
                    let info = fence_info
                        .strip_prefix(FENCE_TAG)
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let node = Node::ActionBlock(ActionBlock {
                        id: None,
                        info,
                        body: text.clone(),
                        span,
                    });
                    push_node(&mut chapters, Some(node), span, diags);
                }
                state = State::Idle;
                text.clear();
            }
            Event::Text(t) | Event::Code(t) => {
                if !matches!(state, State::Idle) {
                    text.push_str(&t);
                }
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                if let Some(node) = directive_from_html(&h) {
                    push_node(&mut chapters, Some(node), span, diags);
                }
            }
            _ => {}
        }
    }

    chapters
}

enum State {
    Idle,
    Heading,
    Paragraph,
    ActionBlock,
}

fn paragraph_node(raw: &str, span: SourceSpan) -> Option<Node> {
    if let Some(node) = directive_from_html(raw) {
        return Some(node);
    }
    let (text, raw_attrs) = split_attr_suffix(raw);
    Some(Node::Segment(Segment {
        id: None,
        text: text.trim().to_string(),
        raw_attrs,
        span,
    }))
}

/// Splits a trailing `{...}` attribute suffix off a paragraph.
fn split_attr_suffix(raw: &str) -> (&str, String) {
    let trimmed = raw.trim_end();
    if !trimmed.ends_with('}') {
        return (raw, String::new());
    }
    match trimmed.rfind('{') {
        Some(open) => (
            &trimmed[..open],
            trimmed[open + 1..trimmed.len() - 1].to_string(),
        ),
        None => (raw, String::new()),
    }
}

fn directive_from_html(html: &str) -> Option<Node> {
    let inner = html
        .trim()
        .strip_prefix("<!--")?
        .strip_suffix("-->")?
        .trim();
    let rest = inner.strip_prefix("teleprompt:")?.trim();
    let ms = rest
        .strip_prefix("pause")?
        .trim()
        .strip_suffix("ms")?
        .trim();
    ms.parse()
        .ok()
        .map(|n| Node::Directive(Directive::Pause(n)))
}

fn push_node(
    chapters: &mut [Chapter],
    node: Option<Node>,
    span: SourceSpan,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(node) = node else { return };
    match chapters.last_mut() {
        Some(ch) => ch.nodes.push(node),
        None => diags.push(
            Diagnostic::error("content appears before the first heading")
                .at(span)
                .with_help("every segment and action block must belong to a chapter"),
        ),
    }
}
