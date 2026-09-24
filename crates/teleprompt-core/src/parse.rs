use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::ast::{slugify, ActionBlock, Chapter, Directive, IdOrigin, Line, Node, Script};
use crate::{Diagnostic, Diagnostics, SourceSpan};

const FENCE_TAG: &str = "teleprompt";

pub fn parse_script(src: &str) -> Result<Script, Diagnostics> {
    let (front_matter, body, body_offset) = match split_front_matter(src) {
        Ok(v) => v,
        Err(d) => return Err(Diagnostics(vec![d])),
    };
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

fn split_front_matter(src: &str) -> Result<(String, &str, usize), Diagnostic> {
    let Some(rest) = src.strip_prefix("---\n") else {
        return Ok((String::new(), src, 0));
    };
    match rest.find("\n---\n") {
        Some(end) => {
            let fm = rest[..end].to_string();
            let after = &rest[end + 5..];
            let lines = 1 + rest[..end + 5].lines().count();
            Ok((fm, after, lines))
        }
        None => Err(Diagnostic::error(
            "the document opens with `---`, so it is read as front matter, but no closing `---` line was found",
        )
        .at(SourceSpan {
            line: 1,
            column: 1,
            len: 3,
        })
        .with_help(
            "add a closing `---` line after the front matter block; a `---` on the first \
             line is always read as a front-matter opener, so a horizontal rule there needs \
             to move or be removed",
        )),
    }
}

fn parse_body(body: &str, line_offset: usize, diags: &mut Vec<Diagnostic>) -> Vec<Chapter> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(body, opts).into_offset_iter();

    let mut chapters: Vec<Chapter> = Vec::new();
    let mut chapter_configured: Vec<bool> = Vec::new();
    let mut state = State::Idle;
    let mut text = String::new();
    let mut fence_info = String::new();
    // Byte offset in `text` just past the most recent inline code span of the
    // paragraph being accumulated. See `split_attr_suffix`.
    let mut code_span_end: Option<usize> = None;

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
                    front_matter: String::new(),
                });
                chapter_configured.push(false);
                state = State::Idle;
                text.clear();
            }
            Event::Start(Tag::Paragraph) => {
                state = State::Paragraph;
                text.clear();
                code_span_end = None;
            }
            Event::End(TagEnd::Paragraph) => {
                // Does the paragraph's last inline code span run all the way
                // to its end? If so, a trailing `}` belongs to that code
                // span, not to an attribute suffix.
                let ends_in_code = code_span_end == Some(text.trim_end().len());
                let raw = text.trim().to_string();
                if !raw.is_empty() {
                    let node = paragraph_node(&raw, ends_in_code, span, diags);
                    push_node(&mut chapters, node, span, diags);
                }
                state = State::Idle;
                text.clear();
                code_span_end = None;
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                fence_info = info.to_string();
                let words: Vec<&str> = fence_info.split_whitespace().collect();
                state = if words.first() == Some(&"yaml") && words.get(1) == Some(&FENCE_TAG) {
                    State::ChapterConfig
                } else if words.first() == Some(&FENCE_TAG) {
                    State::ActionBlock
                } else {
                    State::Idle
                };
                text.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                if matches!(state, State::ChapterConfig) {
                    let already_configured = chapter_configured.last().copied().unwrap_or(false);
                    match chapters.last_mut() {
                        Some(ch) if already_configured => diags.push(
                            Diagnostic::error(format!(
                                "chapter `{}` already has front matter; a second `yaml teleprompt` block is not allowed",
                                ch.slug
                            ))
                            .at(span),
                        ),
                        Some(ch) if ch.nodes.is_empty() => {
                            ch.front_matter = text.clone();
                            if let Some(flag) = chapter_configured.last_mut() {
                                *flag = true;
                            }
                        }
                        Some(_) => diags.push(
                            Diagnostic::error(
                                "chapter configuration must come directly after the heading",
                            )
                            .at(span),
                        ),
                        None => diags.push(
                            Diagnostic::error(
                                "chapter configuration appears before the first heading",
                            )
                            .at(span),
                        ),
                    }
                    state = State::Idle;
                    text.clear();
                    continue;
                }
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
            Event::Text(t) => {
                if !matches!(state, State::Idle) {
                    text.push_str(&t);
                }
            }
            // An inline code span contributes its text like any other run —
            // its content is spoken, so it has to reach `text` — but where it
            // ended is remembered, because that is the one thing that
            // distinguishes `` `{ fps: 30 }` `` from a `{key=value}` suffix
            // once the backticks are gone.
            Event::Code(t) => {
                if !matches!(state, State::Idle) {
                    text.push_str(&t);
                    if matches!(state, State::Paragraph) {
                        code_span_end = Some(text.len());
                    }
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if !matches!(state, State::Idle)
                    && !text.is_empty()
                    && !text.ends_with(char::is_whitespace)
                {
                    text.push(' ');
                }
            }
            Event::Html(h) | Event::InlineHtml(h) => match directive_from_html(&h) {
                Ok(Some(node)) => push_node(&mut chapters, Some(node), span, diags),
                Ok(None) => {}
                Err(message) => diags.push(Diagnostic::error(message).at(span)),
            },
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
    ChapterConfig,
}

fn paragraph_node(
    raw: &str,
    ends_in_code: bool,
    span: SourceSpan,
    diags: &mut Vec<Diagnostic>,
) -> Option<Node> {
    match directive_from_html(raw) {
        Ok(Some(node)) => return Some(node),
        Ok(None) => {}
        Err(message) => {
            diags.push(Diagnostic::error(message).at(span));
            return None;
        }
    }
    let (text, raw_attrs) = split_attr_suffix(raw, ends_in_code);
    let id = raw_attrs
        .split_whitespace()
        .next()
        .and_then(|t| t.strip_prefix('#'))
        .map(str::to_string);
    Some(Node::Line(Line {
        id,
        id_origin: IdOrigin::Derived,
        text: text.trim().to_string(),
        raw_attrs,
        span,
    }))
}

/// Splits a trailing `{...}` attribute suffix off a paragraph.
///
/// `raw` is normalised text with backticks gone, so a paragraph ending in
/// the code span `` `{ fps: 30 }` `` looks like an attribute suffix.
/// `ends_in_code` says the paragraph ended inside a code span, which makes
/// the braces content. Deciding by that rather than by what the braces
/// contain keeps a malformed suffix like `{polcy hold}` an error.
fn split_attr_suffix(raw: &str, ends_in_code: bool) -> (&str, String) {
    if ends_in_code {
        return (raw, String::new());
    }
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

/// Parses an HTML comment as a `teleprompt:` directive.
///
/// `Ok(None)` means the comment is not a teleprompt directive at all (an
/// ordinary HTML comment, legitimately authored, stays silent). Once a
/// comment carries the `teleprompt:` prefix the author's intent is
/// unambiguous, so any failure past that point is `Err` naming the problem
/// rather than a silently dropped directive.
fn directive_from_html(html: &str) -> Result<Option<Node>, String> {
    let Some(inner) = html
        .trim()
        .strip_prefix("<!--")
        .and_then(|s| s.strip_suffix("-->"))
    else {
        return Ok(None);
    };
    let inner = inner.trim();
    let Some(rest) = inner.strip_prefix("teleprompt:") else {
        return Ok(None);
    };
    let rest = rest.trim();

    let Some(arg) = rest.strip_prefix("pause") else {
        let name = rest.split_whitespace().next().unwrap_or(rest);
        return Err(format!(
            "unknown teleprompt directive `{name}`; expected `pause <N>ms`"
        ));
    };
    let arg = arg.trim();
    let bad_value = || {
        format!("invalid pause value `{arg}` in teleprompt directive; expected e.g. `pause 500ms`")
    };
    let Some(digits) = arg.strip_suffix("ms") else {
        return Err(bad_value());
    };
    match digits.trim().parse::<u64>() {
        Ok(n) => Ok(Some(Node::Directive(Directive::Pause(n)))),
        Err(_) => Err(bad_value()),
    }
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
                .with_help("every line and action block must belong to a chapter"),
        ),
    }
}
