//! The retake queue: after the prose is edited, the lines reworded since
//! their takes, recorded again one after another. Each is kept once the
//! reader has moved past it, and the next starts.

use crate::api::{Position, Script};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    /// Each reworded line, and its word count.
    lines: Vec<(usize, usize)>,
    /// How many lines the script has.
    count: usize,
    at: usize,
}

impl Queue {
    /// The script's reworded lines, in order.
    pub fn of(script: &Script) -> Self {
        Self {
            lines: script
                .lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.stale)
                .map(|(i, l)| (i, l.words().count()))
                .collect(),
            count: script.lines.len(),
            at: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The line to record now; `None` once every one is.
    pub fn line(&self) -> Option<usize> {
        self.lines.get(self.at).map(|&(line, _)| line)
    }

    /// Whether the reader, now at `at`, has read the line in hand: moved
    /// on past it, or, on the script's last line, reached its end.
    pub fn read(&self, at: Position) -> bool {
        let Some(&(line, words)) = self.lines.get(self.at) else {
            return false;
        };
        at.line > line || (line + 1 == self.count && at.line == line && at.word >= words)
    }

    /// On to the next reworded line, if there is one.
    pub fn advance(&mut self) -> Option<usize> {
        self.at += 1;
        self.line()
    }

    /// Where the queue is, as the app says it: "1 of 3".
    pub fn says(&self) -> String {
        format!("{} of {}", (self.at + 1).min(self.len()), self.len())
    }
}
