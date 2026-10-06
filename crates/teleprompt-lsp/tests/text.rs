//! Positions as editors count them (lines from 0, UTF-16 code units along
//! a line) against the byte offsets and spans the compiler reports.

use lsp_types::{Position, Range};
use teleprompt_core::SourceSpan;
use teleprompt_lsp::text::LineIndex;

fn pos(line: u32, character: u32) -> Position {
    Position { line, character }
}

#[test]
fn offsets_and_positions_round_trip_through_wide_characters() {
    // "é" is two bytes and one UTF-16 unit; "🎬" four bytes and two units.
    let text = "# Café\n\nAction 🎬 now. {#a}\n";
    let index = LineIndex::new(text);
    let at = text.find("now").unwrap();
    assert_eq!(index.position(at), pos(2, 10));
    assert_eq!(index.offset(pos(2, 10)), at);
    assert_eq!(index.position(text.find('é').unwrap() + 2), pos(0, 6));
    // Past a line's end is its end; past the text, the text's end.
    assert_eq!(index.offset(pos(0, 99)), "# Café".len());
    assert_eq!(index.offset(pos(99, 0)), text.len());
}

#[test]
fn a_span_is_a_range_from_its_column_for_its_bytes() {
    let text = "# Café\n\nAction 🎬 now. {#a}\n";
    let index = LineIndex::new(text);
    // As the parser writes them: line and column from 1, column in
    // characters, length in bytes.
    let span = SourceSpan {
        line: 3,
        column: 8,
        len: "🎬 now.".len(),
    };
    assert_eq!(
        index.range(span),
        Range {
            start: pos(2, 7),
            end: pos(2, 14),
        }
    );
}
