//! What the prompter shows, driven by the server's messages. No I/O: the
//! window applies messages and reads the state back.

use std::collections::BTreeSet;

use std::time::Instant;

use crate::api::{Position, Script, ServerMessage};
use crate::take::Take;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrompterState {
    pub script: Script,
    /// The next word to be said.
    pub at: Position,
    /// Where the take is: counting down, running, paused, or none.
    pub take: Take,
    /// The shot on screen, and the ones to play after it.
    pub playing: Option<String>,
    pub queue: Vec<String>,
    /// Shots that have started this take.
    pub started: BTreeSet<String>,
    pub status: Status,
    /// The last take kept lines, which an undo puts back as they were.
    pub undoable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    pub text: String,
    pub is_error: bool,
}

impl Status {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

/// How a word is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Word {
    Said,
    Next,
    Ahead,
}

impl PrompterState {
    pub fn load(&mut self, script: Script) {
        self.script = script;
        self.at = Position::default();
        self.status = Status::info("Press Ctrl+Shift+Space to record from here, or click a line");
    }

    /// A take is starting at line `from`: nothing has played in it yet.
    pub fn start_take(&mut self, from: usize, now: Instant) {
        self.at = Position {
            line: from,
            word: 0,
        };
        self.playing = None;
        self.queue.clear();
        self.started.clear();
        self.undoable = false;
        self.take.start(now);
        self.status = Status::info(format!("Recording from line {}", from + 1));
    }

    pub fn apply(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::Reached { at, play } => {
                self.at = at;
                if let Some((first, rest)) = play.split_first() {
                    // Reading on cuts off the shot that is playing.
                    self.playing = Some(first.clone());
                    self.queue = rest.to_vec();
                    self.started.extend(play.iter().cloned());
                }
                if at.line >= self.script.lines.len() && !self.script.lines.is_empty() {
                    self.status = Status::info("End of script");
                }
            }
            ServerMessage::Stopped { saved } => {
                // It can come after the next countdown began: that stays.
                if self.take.is_under_way() {
                    self.take.stop(Instant::now());
                }
                self.undoable = !saved.is_empty();
                self.status = Status::info(if saved.is_empty() {
                    "Nothing kept: a line is kept once it is read to its end".to_string()
                } else {
                    format!("Kept {}", self.line_names(&saved))
                });
            }
            ServerMessage::Discarded => {
                if self.take.is_under_way() {
                    self.take.stop(Instant::now());
                }
                self.status = Status::info("Take discarded: nothing kept");
            }
            ServerMessage::Undone { lines } => {
                self.undoable = false;
                self.status = Status::info(if lines.is_empty() {
                    "Nothing to put back".to_string()
                } else {
                    let was = if lines.len() == 1 {
                        "it was"
                    } else {
                        "they were"
                    };
                    format!("Put back {} as {was}", self.line_names(&lines))
                });
            }
            ServerMessage::Error(message) => self.status = Status::error(message),
            ServerMessage::Unknown(_) => {}
        }
    }

    /// Line ids as the reader counts them: "line 2", "lines 1, 3".
    pub fn line_names(&self, ids: &[String]) -> String {
        let numbers: Vec<String> = ids
            .iter()
            .filter_map(|id| self.script.lines.iter().position(|l| &l.id == id))
            .map(|n| (n + 1).to_string())
            .collect();
        match numbers.as_slice() {
            [one] => format!("line {one}"),
            many => format!("lines {}", many.join(", ")),
        }
    }

    /// The shot on screen ended; the next one queued plays.
    pub fn clip_ended(&mut self) {
        self.playing = if self.queue.is_empty() {
            None
        } else {
            Some(self.queue.remove(0))
        };
    }

    pub fn word(&self, line: usize, word: usize) -> Word {
        let here = Position { line, word };
        match here.cmp(&self.at) {
            std::cmp::Ordering::Less => Word::Said,
            std::cmp::Ordering::Equal => Word::Next,
            std::cmp::Ordering::Greater => Word::Ahead,
        }
    }

    /// The clip path of the shot on screen, if it was captured.
    pub fn playing_clip(&self) -> Option<&str> {
        let playing = self.playing.as_deref()?;
        self.script
            .shots
            .iter()
            .find(|s| s.shot == playing)?
            .clip
            .as_deref()
    }
}
