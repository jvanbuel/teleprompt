//! A script's outline: its chapters, and in each its lines and blocks,
//! by the ids `plan` and the prompters show.

use lsp_types::{DocumentSymbol, Range, SymbolKind};
use teleprompt_core::SourceSpan;
use teleprompt_script::ast::Node;
use teleprompt_script::parse::parse_script;

use crate::text::LineIndex;

/// The chapters of `text`, each with its lines and blocks; nothing for a
/// script that does not parse.
pub fn outline(text: &str) -> Vec<DocumentSymbol> {
    let Ok(script) = parse_script(text) else {
        return Vec::new();
    };
    let index = LineIndex::new(text);
    script
        .chapters
        .iter()
        .map(|chapter| {
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
            // The whole heading line, where the parser read it.
            let heading = chapter.span.line.saturating_sub(1);
            let head = index.range(SourceSpan {
                line: chapter.span.line,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Each chapter's heading line, from 0, as the outline puts it.
    fn heading_lines(text: &str) -> Vec<(String, u32)> {
        outline(text)
            .into_iter()
            .map(|c| (c.name, c.selection_range.start.line))
            .collect()
    }

    /// Lines that start with `#` but are not headings, as the parser
    /// reads them, must not move a chapter onto them.
    #[test]
    fn chapters_sit_on_the_headings_the_parser_read() {
        let text = "\
# One

Hello there. {#a}

~~~
# not a heading
~~~

#hashtag is not a heading either. {#b}

# Two

Goodbye. {#c}
";
        assert_eq!(
            heading_lines(text),
            [("One".to_string(), 0), ("Two".to_string(), 10)]
        );
    }
}
