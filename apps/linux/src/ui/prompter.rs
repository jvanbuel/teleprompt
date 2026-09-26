//! The script as a prompter: a text view with each word styled by where the
//! reader is, a mark where each shot starts, and one on recorded lines.

use gtk::prelude::*;
use teleprompt_gtk::api::Position;
use teleprompt_gtk::state::PrompterState;

/// Where each line and word is in the buffer, as character offsets.
#[derive(Default)]
pub struct Layout {
    /// Each line's range, marks included.
    pub lines: Vec<(i32, i32)>,
    /// Each line's words' ranges.
    pub words: Vec<Vec<(i32, i32)>>,
}

impl Layout {
    /// The line at buffer offset `offset`.
    pub fn line_at(&self, offset: i32) -> Option<usize> {
        self.lines
            .iter()
            .position(|&(start, end)| (start..=end).contains(&offset))
    }

    /// The range of the next word, or the buffer's end past the last line.
    fn next(&self, at: Position) -> Option<(i32, i32)> {
        let line = self.words.get(at.line)?;
        line.get(at.word)
            .copied()
            .or_else(|| self.lines.get(at.line).map(|&(_, end)| (end, end)))
    }
}

pub fn tags(buffer: &gtk::TextBuffer) {
    let rgba = |s: &str| gtk::gdk::RGBA::parse(s).expect("a colour");
    buffer.create_tag(
        Some("said"),
        &[("foreground-rgba", &rgba("rgba(255,255,255,0.35)"))],
    );
    buffer.create_tag(
        Some("next"),
        &[
            ("foreground-rgba", &rgba("#ffd84d")),
            ("underline", &gtk::pango::Underline::Single),
        ],
    );
    for (name, colour) in [
        ("shot", "#5fd7ff"),
        ("shot-started", "#808080"),
        ("shot-missing", "#ff6b6b"),
    ] {
        buffer.create_tag(
            Some(name),
            &[
                ("foreground-rgba", &rgba(colour)),
                ("family", &"monospace"),
                ("scale", &0.4f64),
                ("weight", &700i32),
            ],
        );
    }
    buffer.create_tag(Some("recorded"), &[("foreground-rgba", &rgba("#4cd964"))]);
    buffer.create_tag(
        Some("unrecorded"),
        &[("foreground-rgba", &rgba("rgba(0,0,0,0)"))],
    );
}

/// Writes the script into `buffer`; where each line and word landed.
pub fn render(buffer: &gtk::TextBuffer, state: &PrompterState) -> Layout {
    buffer.set_text("");
    let mut layout = Layout::default();
    let mut end = buffer.end_iter();
    for (l, line) in state.script.lines.iter().enumerate() {
        let start = end.offset();
        let mark = if line.recorded {
            "recorded"
        } else {
            "unrecorded"
        };
        buffer.insert_with_tags_by_name(&mut end, "▌ ", &[mark]);
        let mut words = Vec::new();
        let count = line.words().count();
        for (w, word) in line.words().enumerate() {
            markers(buffer, &mut end, state, Position { line: l, word: w });
            let from = end.offset();
            buffer.insert(&mut end, word);
            words.push((from, end.offset()));
            buffer.insert(&mut end, " ");
        }
        markers(
            buffer,
            &mut end,
            state,
            Position {
                line: l,
                word: count,
            },
        );
        layout.lines.push((start, end.offset()));
        layout.words.push(words);
        buffer.insert(&mut end, "\n\n");
    }
    layout
}

fn markers(buffer: &gtk::TextBuffer, end: &mut gtk::TextIter, state: &PrompterState, at: Position) {
    for shot in state.script.shots.iter().filter(|s| s.at == at) {
        let (text, tag) = match &shot.clip {
            None => (format!("▶ {} (not captured) ", shot.shot), "shot-missing"),
            Some(_) if state.started.contains(&shot.shot) => {
                (format!("▶ {} ", shot.shot), "shot-started")
            }
            Some(_) => (format!("▶ {} ", shot.shot), "shot"),
        };
        buffer.insert_with_tags_by_name(end, &text, &[tag]);
    }
}

/// Styles the words said and the next one, and scrolls it a third of the
/// way down.
pub fn show_position(view: &gtk::TextView, layout: &Layout, at: Position) {
    let buffer = view.buffer();
    let (start, end) = buffer.bounds();
    buffer.remove_tag_by_name("said", &start, &end);
    buffer.remove_tag_by_name("next", &start, &end);
    let Some((from, to)) = layout.next(at).or_else(|| {
        // Past the last line: all of it said.
        layout.lines.last().map(|&(_, end)| (end, end))
    }) else {
        return;
    };
    buffer.apply_tag_by_name("said", &start, &buffer.iter_at_offset(from));
    buffer.apply_tag_by_name(
        "next",
        &buffer.iter_at_offset(from),
        &buffer.iter_at_offset(to),
    );
    let mark = buffer
        .mark("reader")
        .unwrap_or_else(|| buffer.create_mark(Some("reader"), &start, true));
    buffer.move_mark(&mark, &buffer.iter_at_offset(from));
    view.scroll_to_mark(&mark, 0.0, true, 0.0, 0.3);
}
