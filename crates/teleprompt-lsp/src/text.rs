//! Positions as editors count them against offsets and spans as the
//! compiler does. An editor counts lines from 0 and, along a line, UTF-16
//! code units; the compiler reports lines and columns from 1, columns in
//! characters and lengths in bytes.

use lsp_types::{Position, Range};
use teleprompt_core::SourceSpan;

/// Where each line of a text starts.
pub struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self { text, starts }
    }

    /// The line `offset` is on, from 0, and where that line starts.
    fn line_of(&self, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        (line, self.starts[line])
    }

    /// The text of line `line`, from 0, without its newline.
    pub fn line(&self, line: usize) -> &'a str {
        let Some(&start) = self.starts.get(line) else {
            return "";
        };
        let end = self
            .starts
            .get(line + 1)
            .map_or(self.text.len(), |&e| e - 1);
        &self.text[start..end.max(start)]
    }

    pub fn position(&self, offset: usize) -> Position {
        let offset = offset.min(self.text.len());
        let (line, start) = self.line_of(offset);
        let character: usize = self.text[start..offset].chars().map(char::len_utf16).sum();
        Position {
            line: line as u32,
            character: character as u32,
        }
    }

    /// The byte offset of `position`: past its line's end, the end; past
    /// the last line, the text's end.
    pub fn offset(&self, position: Position) -> usize {
        let Some(&start) = self.starts.get(position.line as usize) else {
            return self.text.len();
        };
        let mut units = 0;
        for (i, c) in self.line(position.line as usize).char_indices() {
            if units >= position.character as usize {
                return start + i;
            }
            units += c.len_utf16();
        }
        start + self.line(position.line as usize).len()
    }

    /// Where `span` is, from its column for its length.
    pub fn range(&self, span: SourceSpan) -> Range {
        let line = span.line.saturating_sub(1);
        let text = self.line(line);
        let column = text
            .char_indices()
            .nth(span.column.saturating_sub(1))
            .map_or(text.len(), |(i, _)| i);
        let start = self
            .starts
            .get(line)
            .map_or(self.text.len(), |&s| s + column);
        Range {
            start: self.position(start),
            end: self.position(start + span.len),
        }
    }
}
