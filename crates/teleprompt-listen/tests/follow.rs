use std::collections::VecDeque;
use teleprompt_listen::{Follower, Heard, Position, Recognizer};

/// A recognizer that hears what the test says it heard, one hypothesis per
/// chunk of audio.
struct Scripted(VecDeque<Heard>);

impl Recognizer for Scripted {
    fn listen(&mut self, _samples: &[f32]) -> Heard {
        self.0.pop_front().expect("the test ran out of hypotheses")
    }
}

fn partial(text: &str) -> Heard {
    Heard {
        text: text.to_string(),
        is_final: false,
    }
}

const SCRIPT: &[&str] = &[
    "Welcome to Acme. Let me show you around.",
    "Deployment is one command.",
];

#[test]
fn audio_that_moves_the_reader_reports_where_to() {
    let heard = Scripted(VecDeque::from([partial("welcome to")]));
    let mut f = Follower::new(heard, SCRIPT);
    assert_eq!(f.listen(&[0.0; 160]), Some(Position { line: 0, word: 2 }));
}

/// Most chunks of audio change nothing, and the prompter only needs news.
#[test]
fn audio_that_leaves_the_reader_put_reports_nothing() {
    let heard = Scripted(VecDeque::from([
        partial("welcome to"),
        partial("welcome to"),
    ]));
    let mut f = Follower::new(heard, SCRIPT);
    f.listen(&[0.0; 160]);
    assert_eq!(f.listen(&[0.0; 160]), None);
}

/// When the recognizer finishes an utterance the reader stays where it
/// ended, so the next one, a single word, carries straight on.
#[test]
fn a_finished_utterance_is_where_the_next_one_starts() {
    let heard = Scripted(VecDeque::from([
        Heard {
            text: "welcome to acme".to_string(),
            is_final: true,
        },
        partial("let"),
    ]));
    let mut f = Follower::new(heard, SCRIPT);
    f.listen(&[0.0; 160]);
    assert_eq!(f.listen(&[0.0; 160]), Some(Position { line: 0, word: 4 }));
}
