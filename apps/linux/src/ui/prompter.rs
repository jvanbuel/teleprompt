//! The script as text: each word styled by where the reader is, and a small
//! diamond where each shot starts, so the prose reads uninterrupted.

use gtk::prelude::*;
use teleprompt_gtk::api::Position;
use teleprompt_gtk::state::{PrompterState, Word};

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

    /// The range of the next word; past a line's last word, its end; past
    /// the last line, the end of the script.
    pub fn next(&self, at: Position) -> Option<(i32, i32)> {
        match self.words.get(at.line) {
            Some(line) => line
                .get(at.word)
                .copied()
                .or_else(|| self.lines.get(at.line).map(|&(_, end)| (end, end))),
            None => self.lines.last().map(|&(_, end)| (end, end)),
        }
    }
}

pub fn tags(buffer: &gtk::TextBuffer) {
    let rgba = |s: &str| gtk::gdk::RGBA::parse(s).expect("a colour");
    let ink = |alpha: f32| {
        let mut c = rgba("#f2f4f7");
        c.set_alpha(alpha);
        c
    };
    buffer.create_tag(Some("said"), &[("foreground-rgba", &ink(0.3))]);
    buffer.create_tag(Some("later"), &[("foreground-rgba", &ink(0.55))]);
    buffer.create_tag(
        Some("next"),
        &[
            ("foreground-rgba", &rgba("#ffb800")),
            ("underline", &gtk::pango::Underline::Single),
            ("underline-rgba", &rgba("#ffb800")),
        ],
    );
    for (name, colour) in [
        ("shot", ink(0.6)),
        ("shot-aired", ink(0.22)),
        ("shot-missing", rgba("#f28b82")),
    ] {
        buffer.create_tag(
            Some(name),
            &[
                ("foreground-rgba", &colour),
                ("scale", &0.5f64),
                ("rise", &(6 * gtk::pango::SCALE)),
            ],
        );
    }
}

/// Writes the script into `buffer`; where each line and word landed.
pub fn render(buffer: &gtk::TextBuffer, state: &PrompterState) -> Layout {
    buffer.set_text("");
    let mut layout = Layout::default();
    let mut end = buffer.end_iter();
    for (l, line) in state.script.lines.iter().enumerate() {
        let start = end.offset();
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
        if l + 1 < state.script.lines.len() {
            buffer.insert(&mut end, "\n");
        }
    }
    layout
}

fn markers(buffer: &gtk::TextBuffer, end: &mut gtk::TextIter, state: &PrompterState, at: Position) {
    for shot in state.script.shots.iter().filter(|s| s.at == at) {
        let tag = match &shot.clip {
            None => "shot-missing",
            Some(_) if state.started.contains(&shot.shot) => "shot-aired",
            Some(_) => "shot",
        };
        buffer.insert_with_tags_by_name(end, "◆", &[tag]);
        buffer.insert(end, " ");
    }
}

/// Dims what is said and what lies beyond the reader's line, and marks the
/// next word. The next word's start, to glide to.
pub fn show_position(
    buffer: &gtk::TextBuffer,
    layout: &Layout,
    state: &PrompterState,
) -> Option<i32> {
    let (start, end) = buffer.bounds();
    for tag in ["said", "next", "later"] {
        buffer.remove_tag_by_name(tag, &start, &end);
    }
    let (from, to) = layout.next(state.at)?;
    buffer.apply_tag_by_name("said", &start, &buffer.iter_at_offset(from));
    buffer.apply_tag_by_name(
        "next",
        &buffer.iter_at_offset(from),
        &buffer.iter_at_offset(to),
    );
    if let Some(&(_, line_end)) = layout.lines.get(state.at.line) {
        buffer.apply_tag_by_name("later", &buffer.iter_at_offset(line_end), &end);
    }
    debug_assert!(matches!(
        state.word(state.at.line, state.at.word),
        Word::Next
    ));
    Some(from)
}
