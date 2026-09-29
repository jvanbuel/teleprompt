use crate::{Aligner, Position};

/// What a streaming recognizer has heard of the utterance under way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heard {
    pub text: String,
    /// The recognizer finished the utterance; the next starts afresh.
    pub is_final: bool,
}

/// A streaming speech recognizer: mono samples in, at the rate it was made
/// for, and its hypothesis for the utterance under way out.
pub trait Recognizer {
    fn listen(&mut self, samples: &[f32]) -> Heard;

    /// Forgets everything heard, for a new take.
    fn reset(&mut self);
}

/// A recognizer that hears nothing: for a prompter whose script is read
/// by a synthesized voice, not followed by ear.
pub struct Deaf;

impl Recognizer for Deaf {
    fn listen(&mut self, _samples: &[f32]) -> Heard {
        Heard {
            text: String::new(),
            is_final: false,
        }
    }

    fn reset(&mut self) {}
}

/// A reader followed through a script by ear.
pub struct Follower<R> {
    recognizer: R,
    aligner: Aligner,
    /// Where the reader was last reported to be.
    reported: Option<Position>,
}

impl<R: Recognizer> Follower<R> {
    pub fn new(recognizer: R, lines: &[&str]) -> Self {
        Self {
            recognizer,
            aligner: Aligner::new(lines),
            reported: None,
        }
    }

    /// The script's lines as they now read, after an edit; the reader is
    /// back at the top.
    pub fn set_lines(&mut self, lines: &[&str]) {
        self.aligner = Aligner::new(lines);
        self.recognizer.reset();
        self.reported = None;
    }

    /// A new take: the reader is back at the top, and nothing heard before
    /// carries into it.
    pub fn restart(&mut self) {
        self.restart_at(0);
    }

    /// A new take from `line`, to read it again.
    pub fn restart_at(&mut self, line: usize) {
        self.recognizer.reset();
        self.aligner.restart_at(line);
        self.reported = None;
    }

    /// Every word in `samples` (at the recognizer's rate), heard afresh:
    /// what a line's take says. The reader is back at the top after it.
    pub fn transcribe(&mut self, samples: &[f32]) -> String {
        const CHUNK: usize = 1600;
        self.recognizer.reset();
        // Silence after the end, so the last word is heard out.
        let tail = [0.0; CHUNK];
        let mut said = Vec::new();
        let mut pending = String::new();
        for chunk in samples
            .chunks(CHUNK)
            .chain(std::iter::repeat_n(&tail[..], 10))
        {
            let heard = self.recognizer.listen(chunk);
            if heard.is_final {
                said.push(heard.text);
                pending.clear();
            } else {
                pending = heard.text;
            }
        }
        said.push(pending);
        self.restart();
        said.iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Feeds a chunk of audio; where the reader now is, if they moved.
    pub fn listen(&mut self, samples: &[f32]) -> Option<Position> {
        let heard = self.recognizer.listen(samples);
        let now = self.aligner.hear(&heard.text);
        if heard.is_final {
            self.aligner.commit();
        }
        if self.reported == Some(now) {
            return None;
        }
        self.reported = Some(now);
        Some(now)
    }
}
