//! The protocol: open documents kept in full, analyzed on each change, and
//! requests answered from the last analysis and the project.

use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;

use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::{
    CompletionOptions, Diagnostic as LspDiagnostic, DiagnosticSeverity, GotoDefinitionResponse,
    Hover, HoverContents, HoverProviderCapability, Location, MarkupContent, MarkupKind, OneOf,
    Position, Range, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use serde_json::Value;
use teleprompt_core::ast::{slugify, Node};
use teleprompt_core::parse::parse_script;
use teleprompt_core::Severity;

use crate::text::LineIndex;
use crate::{complete, symbols, Analysis, Analyzer, Definition, Project};

type Failure = Box<dyn Error + Send + Sync>;

/// Serves on stdin and stdout until the editor says to exit.
pub fn run(analyzer: impl Analyzer) -> Result<(), Failure> {
    let (connection, io) = Connection::stdio();
    serve(connection, analyzer)?;
    io.join()?;
    Ok(())
}

/// Serves on `connection` until the editor says to exit.
pub fn serve(connection: Connection, analyzer: impl Analyzer) -> Result<(), Failure> {
    connection.initialize(serde_json::to_value(capabilities())?)?;
    let mut server = Server {
        analyzer,
        documents: HashMap::new(),
    };
    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    return Ok(());
                }
                let id = request.id.clone();
                let result = server.answer(&request).unwrap_or(Value::Null);
                connection
                    .sender
                    .send(Message::Response(Response::new_ok(id, result)))?;
            }
            Message::Notification(notification) => {
                if let Some(published) = server.notice(&notification) {
                    connection.sender.send(Message::Notification(published))?;
                }
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(["@", "=", "{", " "].map(String::from).to_vec()),
            ..CompletionOptions::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    }
}

struct Document {
    text: String,
    analysis: Analysis,
}

struct Server<A> {
    analyzer: A,
    documents: HashMap<Url, Document>,
}

impl<A: Analyzer> Server<A> {
    /// A document opened, changed or closed: its problems, as they now are.
    fn notice(&mut self, n: &Notification) -> Option<Notification> {
        let params = &n.params;
        let uri: Url = serde_json::from_value(params["textDocument"]["uri"].clone()).ok()?;
        let text = match n.method.as_str() {
            "textDocument/didOpen" => params["textDocument"]["text"].as_str()?.to_string(),
            // Whole documents are synced, so the last change is the text.
            "textDocument/didChange" => params["contentChanges"].as_array()?.last()?["text"]
                .as_str()?
                .to_string(),
            "textDocument/didClose" => {
                self.documents.remove(&uri);
                return Some(published(&uri, Vec::new()));
            }
            _ => return None,
        };
        let path = uri.to_file_path().ok()?;
        let analysis = if crate::is_script(&text) {
            self.analyzer.analyze(&path, &text)
        } else {
            Analysis::default()
        };
        let diagnostics = diagnostics(&text, &analysis);
        self.documents
            .insert(uri.clone(), Document { text, analysis });
        Some(published(&uri, diagnostics))
    }

    fn answer(&self, request: &Request) -> Option<Value> {
        let params = &request.params;
        let uri: Url = serde_json::from_value(params["textDocument"]["uri"].clone()).ok()?;
        let document = self.documents.get(&uri)?;
        if !crate::is_script(&document.text) {
            return None;
        }
        let path = uri.to_file_path().ok()?;
        let position: Option<Position> = serde_json::from_value(params["position"].clone()).ok();
        let value = match request.method.as_str() {
            "textDocument/completion" => {
                let (ctx, _) = complete::context(&document.text, position?)?;
                serde_json::to_value(complete::items(&ctx, &self.analyzer.project(&path)))
            }
            "textDocument/hover" => {
                serde_json::to_value(hover(document, &self.analyzer.project(&path), position?)?)
            }
            "textDocument/definition" => serde_json::to_value(definition(
                &document.text,
                &self.analyzer.project(&path),
                position?,
            )?),
            "textDocument/documentSymbol" => serde_json::to_value(symbols::outline(&document.text)),
            _ => return None,
        };
        value.ok()
    }
}

fn published(uri: &Url, diagnostics: Vec<LspDiagnostic>) -> Notification {
    Notification::new(
        "textDocument/publishDiagnostics".into(),
        serde_json::json!({ "uri": uri, "diagnostics": diagnostics }),
    )
}

/// The analysis's problems where they are in `text`. One about another
/// file, such as an included one, is put at the top, naming it.
fn diagnostics(text: &str, analysis: &Analysis) -> Vec<LspDiagnostic> {
    let index = LineIndex::new(text);
    analysis
        .diagnostics
        .iter()
        .map(|d| {
            let range = match (&d.file, d.span) {
                (None, Some(span)) => index.range(span),
                _ => Range::default(),
            };
            let mut message = match &d.file {
                Some(file) => format!("{file}: {}", d.message),
                None => d.message.clone(),
            };
            if let Some(help) = &d.help {
                message.push_str(&format!("\nhelp: {help}"));
            }
            LspDiagnostic {
                range,
                severity: Some(match d.severity {
                    Severity::Error => DiagnosticSeverity::ERROR,
                    Severity::Warning => DiagnosticSeverity::WARNING,
                }),
                source: Some("teleprompt".into()),
                message,
                ..LspDiagnostic::default()
            }
        })
        .collect()
}

/// The run of non-space text around `position`, as `(token, range)`.
fn token_at(text: &str, position: Position) -> Option<(String, Range)> {
    let index = LineIndex::new(text);
    let line = index.line(position.line as usize);
    let start_of_line = index.offset(Position {
        line: position.line,
        character: 0,
    });
    let at = (index.offset(position) - start_of_line).min(line.len());
    let from = line[..at].rfind(char::is_whitespace).map_or(0, |i| i + 1);
    let to = line[at..]
        .find(char::is_whitespace)
        .map_or(line.len(), |i| at + i);
    let token = line[from..to].trim_end_matches('}').trim_start_matches('{');
    (!token.is_empty()).then(|| {
        (
            token.to_string(),
            Range {
                start: index.position(start_of_line + from),
                end: index.position(start_of_line + to),
            },
        )
    })
}

/// The label under the cursor, `**Ada Lovelace:**` as one token, however
/// many words it has.
fn label_at(text: &str, position: Position) -> Option<String> {
    let index = LineIndex::new(text);
    let line = index.line(position.line as usize);
    let mark = ["**", "__"].into_iter().find(|m| line.starts_with(m))?;
    let end = 2 + line[2..].find(mark)? + 2;
    let start = index.offset(Position {
        line: position.line,
        character: 0,
    });
    let label = &line[..end];
    (index.offset(position) - start <= end && label.contains(':')).then(|| label.to_string())
}

/// What the name under the cursor refers to: a speaker, by their label
/// (`**Guest:**`) or a heading's `speaker=`, or a scene.
fn named<'p>(token: &str, project: &'p Project) -> Option<&'p Definition> {
    let bare = token.trim_matches(|c| c == '*' || c == '_');
    let label = (bare.len() < token.len())
        .then(|| bare.strip_suffix(':'))
        .flatten();
    let speaker = label.or_else(|| Some(token.strip_prefix("speaker=")?.trim_matches('"')));
    if let Some(name) = speaker {
        return project
            .speakers
            .iter()
            .find(|d| slugify(&d.name) == slugify(name));
    }
    let name = token.strip_prefix("scene=")?.trim_matches('"');
    project.scenes.iter().find(|d| d.name == name)
}

/// A speaker or scene under the cursor, or else the line or block it is
/// in: what it compiles to.
fn hover(document: &Document, project: &Project, position: Position) -> Option<Hover> {
    let text = &document.text;
    let token = label_at(text, position).or_else(|| token_at(text, position).map(|(t, _)| t));
    let markdown = match token.and_then(|t| named(&t, project).cloned()) {
        Some(def) => format!("**{}** · {}", def.name, def.detail),
        None => {
            let line = node_at(text, position)?;
            document.analysis.hovers.get(&line)?.clone()
        }
    };
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: None,
    })
}

/// The source line (from 1) of the line or block `position` is in.
fn node_at(text: &str, position: Position) -> Option<usize> {
    let script = parse_script(text).ok()?;
    let index = LineIndex::new(text);
    script
        .chapters
        .iter()
        .flat_map(|c| c.nodes.iter())
        .filter_map(|node| match node {
            Node::Line(l) => Some(l.span),
            Node::ActionBlock(b) => Some(b.span),
            Node::Directive(_) => None,
        })
        .find(|span| {
            let range = index.range(*span);
            (range.start.line..=range.end.line).contains(&position.line)
        })
        .map(|span| span.line)
}

/// Where the name or path under the cursor is defined: a speaker in the
/// cast, a scene in the configuration, or an included file.
fn definition(text: &str, project: &Project, position: Position) -> Option<GotoDefinitionResponse> {
    let token = label_at(text, position).or_else(|| token_at(text, position).map(|(t, _)| t))?;
    let (file, line) = match named(&token, project) {
        Some(def) => def.location.clone()?,
        None => {
            let path = token.strip_prefix("include=")?.trim_matches('"');
            let path = path.split('#').next().unwrap_or(path);
            (project.script_dir.join(PathBuf::from(path)), 0)
        }
    };
    let start = Position { line, character: 0 };
    Some(GotoDefinitionResponse::Scalar(Location {
        uri: Url::from_file_path(file).ok()?,
        range: Range { start, end: start },
    }))
}
