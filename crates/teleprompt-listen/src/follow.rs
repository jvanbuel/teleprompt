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
