//! A script's outline: its chapters, and in each its lines and blocks,
//! by the ids `plan` and the prompters show.

use lsp_types::{DocumentSymbol, Range, SymbolKind};
use teleprompt_core::SourceSpan;
use teleprompt_script::ast::Node;
use teleprompt_script::parse::parse_script;

use crate::lsp::text::LineIndex;

/// The chapters of `text`, each with its lines and blocks; nothing for a
/// script that does not parse.
pub fn outline(text: &str) -> Vec<DocumentSymbol> {
    let Ok(script) = parse_script(text) else {
        return Vec::new();
    };
    let index = LineIndex::new(text);
    let headings = headings(text);
    script
        .chapters
        .iter()
        .zip(headings.iter())
        .map(|(chapter, &heading)| {
            let children: Vec<DocumentSymbol> = chapter
                .nodes
                .iter()
                .filter_map(|node| match node {
                    Node::Line(l) => Some(symbol(
                        l.id.to_string(),
                        Some(l.text.clone()),
                        SymbolKind::STRING,
                        index.range(l.span),
                    )),
                    Node::ActionBlock(b) => Some(symbol(
                        b.id.to_string(),
                        Some(b.info.trim().to_string()),
                        SymbolKind::EVENT,
                        index.range(b.span),
                    )),
                    Node::Directive(_) => None,
                })
                .collect();
            let head = index.range(SourceSpan {
                line: heading + 1,
                column: 1,
                len: index.line(heading).len(),
            });
            let end = children.last().map_or(head.end, |c| c.range.end);
            let mut chapter_symbol = symbol(
                chapter.title.clone(),
                None,
                SymbolKind::NAMESPACE,
                Range {
                    start: head.start,
                    end,
                },
            );
            chapter_symbol.selection_range = head;
            chapter_symbol.children = Some(children);
            chapter_symbol
        })
        .collect()
}

#[allow(deprecated)] // `deprecated` is a field every symbol must still name.
fn symbol(name: String, detail: Option<String>, kind: SymbolKind, range: Range) -> DocumentSymbol {
    DocumentSymbol {
        name,
        detail,
        kind,
        tags: None,
        deprecated: None,
        range,
        selection_range: range,
        children: None,
    }
}

/// The lines, from 0, of the chapters' headings: `#` lines outside front
/// matter and fences, as the parser reads them.
fn headings(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut fence = false;
    let mut front = text.starts_with("---");
    for (n, line) in text.lines().enumerate() {
        if front {
            front = n == 0 || line.trim_end() != "---";
            continue;
        }
        if line.trim_start().starts_with("```") {
            fence = !fence;
        } else if !fence && line.starts_with('#') {
            out.push(n);
        }
    }
    out
}
