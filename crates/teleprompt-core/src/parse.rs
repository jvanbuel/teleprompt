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
    let mut b = BodyBuilder {
        chapters: Vec::new(),
        chapter_configured: Vec::new(),
        state: State::Idle,
        text: String::new(),
        fence_info: String::new(),
        code_span_end: None,
        diags,
    };

    for (event, range) in parser {
        let span = span_at(body, line_offset, range.start, range.len());

        match event {
            Event::Start(Tag::Heading { .. }) => b.begin(State::Heading),
            Event::End(TagEnd::Heading(_)) => b.end_heading(),
            Event::Start(Tag::Paragraph) => {
                b.begin(State::Paragraph);
                b.code_span_end = None;
            }
            Event::End(TagEnd::Paragraph) => b.end_paragraph(span),
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => b.start_fence(&info),
            Event::End(TagEnd::CodeBlock) => b.end_code_block(span),
            Event::Text(t) => {
                if !matches!(b.state, State::Idle) {
                    b.text.push_str(&t);
                }
            }
            // An inline code span contributes its text like any other run —
            // its content is spoken, so it has to reach `text` — but where it
            // ended is remembered, because that is the one thing that
            // distinguishes `` `{ fps: 30 }` `` from a `{key=value}` suffix
            // once the backticks are gone.
            Event::Code(t) => {
                if !matches!(b.state, State::Idle) {
                    b.text.push_str(&t);
                    if matches!(b.state, State::Paragraph) {
                        b.code_span_end = Some(b.text.len());
                    }
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if !matches!(b.state, State::Idle)
                    && !b.text.is_empty()
                    && !b.text.ends_with(char::is_whitespace)
                {
                    b.text.push(' ');
                }
            }
            Event::Html(h) | Event::InlineHtml(h) => match directive_from_html(&h) {
                // Its heading's chapter does not exist yet, so it would join
                // the one before.
                Ok(Some(_)) if matches!(b.state, State::Heading) => b.diags.push(
                    Diagnostic::error("a directive goes on its own line, not inside a heading")
                        .at(span)
                        .with_help("move it to a line of its own below the heading"),
                ),
                Ok(Some(node)) => push_node(&mut b.chapters, Some(node), span, b.diags),
                Ok(None) => {}
                Err(message) => b.diags.push(Diagnostic::error(message).at(span)),
            },
            _ => {}
        }
    }

    b.chapters
}

enum State {
    Idle,
    Heading,
    Paragraph,
    ActionBlock,
    ChapterConfig,
}

/// The chapters built so far plus the block currently being read: `state`
/// says what kind of block it is and `text` holds its content.
struct BodyBuilder<'d> {
    chapters: Vec<Chapter>,
    /// Parallel to `chapters`: whether that chapter has taken its
    /// `yaml teleprompt` block.
    chapter_configured: Vec<bool>,
    state: State,
    text: String,
    fence_info: String,
    /// Byte offset in `text` just past the most recent inline code span of
    /// the paragraph being accumulated. See `split_attr_suffix`.
    code_span_end: Option<usize>,
    diags: &'d mut Vec<Diagnostic>,
}

impl BodyBuilder<'_> {
    fn begin(&mut self, state: State) {
        self.state = state;
        self.text.clear();
    }

    /// Opens a new chapter titled by the heading just read.
    fn end_heading(&mut self) {
        let title = self.text.trim().to_string();
        self.chapters.push(Chapter {
            slug: slugify(&title),
            title,
            nodes: Vec::new(),
            front_matter: String::new(),
        });
        self.chapter_configured.push(false);
        self.begin(State::Idle);
    }

    fn end_paragraph(&mut self, span: SourceSpan) {
        // Does the paragraph's last inline code span run all the way to its
        // end? If so, a trailing `}` belongs to that code span, not to an
        // attribute suffix.
        let ends_in_code = self.code_span_end == Some(self.text.trim_end().len());
        let raw = self.text.trim().to_string();
        if !raw.is_empty() {
            let node = paragraph_node(&raw, ends_in_code, span, self.diags);
            push_node(&mut self.chapters, node, span, self.diags);
        }
        self.begin(State::Idle);
        self.code_span_end = None;
    }

    /// Classifies a fenced block by its info string: ` ```yaml teleprompt `
    /// is chapter config, ` ```teleprompt ` an action block, anything else
    /// is ignored.
    fn start_fence(&mut self, info: &str) {
        self.fence_info = info.to_string();
        let words: Vec<&str> = self.fence_info.split_whitespace().collect();
        let state = if words.first() == Some(&"yaml") && words.get(1) == Some(&FENCE_TAG) {
            State::ChapterConfig
        } else if words.first() == Some(&FENCE_TAG) {
            State::ActionBlock
        } else {
            State::Idle
        };
        self.begin(state);
    }

    fn end_code_block(&mut self, span: SourceSpan) {
        match self.state {
            State::ChapterConfig => self.attach_chapter_config(span),
            State::ActionBlock => {
                let info = self
                    .fence_info
                    .strip_prefix(FENCE_TAG)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let node = Node::ActionBlock(ActionBlock {
                    id: None,
                    info,
                    body: self.text.clone(),
                    span,
                });
                push_node(&mut self.chapters, Some(node), span, self.diags);
            }
            _ => {}
        }
        self.begin(State::Idle);
    }

    /// Stores a `yaml teleprompt` block as the current chapter's front
    /// matter, provided it is the first one and nothing precedes it.
    fn attach_chapter_config(&mut self, span: SourceSpan) {
        let already_configured = self.chapter_configured.last().copied().unwrap_or(false);
        match self.chapters.last_mut() {
            Some(ch) if already_configured => self.diags.push(
                Diagnostic::error(format!(
                    "chapter `{}` already has front matter; a second `yaml teleprompt` block is not allowed",
                    ch.slug
                ))
                .at(span),
            ),
            Some(ch) if ch.nodes.is_empty() => {
                ch.front_matter = self.text.clone();
                if let Some(flag) = self.chapter_configured.last_mut() {
                    *flag = true;
                }
            }
            Some(_) => self.diags.push(
                Diagnostic::error("chapter configuration must come directly after the heading")
                    .at(span),
            ),
            None => self.diags.push(
                Diagnostic::error("chapter configuration appears before the first heading")
                    .at(span),
            ),
        }
    }
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
        Ok(n) if n <= crate::attrs::MAX_DURATION_MS => {
            Ok(Some(Node::Directive(Directive::Pause(n))))
        }
        Ok(_) => Err(format!(
            "pause `{arg}` is longer than a day, the most a pause may be"
        )),
        Err(_) => Err(bad_value()),
    }
}

/// Where the byte `start` of `body` is, as the script's line and column.
/// Counts newlines, not `lines()`: an event that starts partway through a
/// line would otherwise count that unfinished line too.
fn span_at(body: &str, line_offset: usize, start: usize, len: usize) -> SourceSpan {
    let before = &body[..start];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    SourceSpan {
        line: line_offset + before.matches('\n').count() + 1,
        column: before[line_start..].chars().count() + 1,
        len,
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
