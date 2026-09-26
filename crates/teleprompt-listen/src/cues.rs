use std::ops::Range;

use crate::Position;

/// Points in a script, in script order, each firing once when the reader
/// reaches it.
pub struct Cues {
    at: Vec<Position>,
    /// How many have fired: the reader passed every cue before this index.
    fired: usize,
}

impl Cues {
    pub fn new(at: Vec<Position>) -> Self {
        Self { at, fired: 0 }
    }

    /// The reader is at `now`; the indices of the cues that newly fired.
    pub fn reach(&mut self, now: Position) -> Range<usize> {
        let from = self.fired;
        while self.at.get(self.fired).is_some_and(|&cue| cue <= now) {
            self.fired += 1;
        }
        from..self.fired
    }

    /// A new take: every cue fires again.
    pub fn restart(&mut self) {
        self.fired = 0;
    }

    /// A new take from `at`: the cues before it stay behind, as fired.
    pub fn restart_at(&mut self, at: Position) {
        self.fired = self.at.partition_point(|&cue| cue < at);
    }
}
