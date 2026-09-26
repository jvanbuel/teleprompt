//! Following a reader through a script by what a speech recognizer hears.

mod cues;
mod follow;
mod take;

pub use cues::Cues;
pub use follow::{Follower, Heard, Recognizer};
pub use take::TakeLog;

/// The next word the reader will say: its line, and its index in that line.
/// Ordered as in the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: usize,
    pub word: usize,
}

/// Where a reader is in a script, from the recognizer's hypotheses.
pub struct Aligner {
    words: Vec<ScriptWord>,
    /// Where the utterance under way started.
    anchor: usize,
    /// How far its latest hypothesis reached.
    reached: usize,
}

struct ScriptWord {
    line: usize,
    word: usize,
    text: String,
}

impl Aligner {
    pub fn new(lines: &[&str]) -> Self {
        let words = lines
            .iter()
            .enumerate()
            .flat_map(|(line, text)| {
                text.split_whitespace()
                    .enumerate()
                    .flat_map(move |(word, written)| {
                        parts(written)
                            .flat_map(|part| spoken(&part))
                            .map(move |text| ScriptWord { line, word, text })
                            .collect::<Vec<_>>()
                    })
            })
            .collect();
        Self {
            words,
            anchor: 0,
            reached: 0,
        }
    }

    /// Aligns the recognizer's current hypothesis for the utterance under
    /// way and returns where the reader now is.
    pub fn hear(&mut self, hypothesis: &str) -> Position {
        let heard: Vec<String> = hypothesis.split_whitespace().flat_map(parts).collect();
        let ahead = &self.words[self.anchor..(self.anchor + WINDOW).min(self.words.len())];
        let matched = self.anchor + best_match_end(&heard, ahead);
        self.reached = matched;
        self.position_of(matched)
    }

    /// The recognizer finalized the utterance: the next starts where it
    /// ended.
    pub fn commit(&mut self) {
        self.anchor = self.reached;
    }

    /// A new take, from the top.
    pub fn restart(&mut self) {
        self.anchor = 0;
        self.reached = 0;
    }

    fn position_of(&self, at: usize) -> Position {
        match self.words.get(at) {
            Some(w) => Position {
                line: w.line,
                word: w.word,
            },
            None => {
                let last = self.words.last().map_or(0, |w| w.line);
                Position {
                    line: last + 1,
                    word: 0,
                }
            }
        }
    }
}

/// How many script words past the anchor a hypothesis is aligned against.
const WINDOW: usize = 50;

/// How many script words past the anchor the reader has reached.
///
/// Aligns `heard` against `script` as a longest common subsequence, so a
/// misheard word, a word not in the script and a skipped one each cost a
/// match rather than the place. The matches are then accepted in order: one
/// that continues the last (at most [`MAX_GAP`] script words on) always, one
/// further ahead only as the start of a run of two, since a single word
/// recurring later in the script is no evidence the reader jumped there.
fn best_match_end(heard: &[String], script: &[ScriptWord]) -> usize {
    let matches = aligned_matches(heard, script);
    let mut reached = 0;
    for (k, &(i, j)) in matches.iter().enumerate() {
        let continues = j <= reached + MAX_GAP + 1;
        let starts_a_run = matches.get(k + 1) == Some(&(i + 1, j + 1));
        if continues || starts_a_run {
            reached = j;
        }
    }
    reached
}

/// Script words that may lie between two matches for the second still to
/// continue the first: room for one misheard word.
const MAX_GAP: usize = 1;

/// The (heard, script) pairs, 1-based and in order, that a longest common
/// subsequence of the two matches up.
fn aligned_matches(heard: &[String], script: &[ScriptWord]) -> Vec<(usize, usize)> {
    let (n, m) = (heard.len(), script.len());
    let same = |i: usize, j: usize| heard[i - 1] == script[j - 1].text;
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            lcs[i][j] = if same(i, j) {
                lcs[i - 1][j - 1] + 1
            } else {
                lcs[i - 1][j].max(lcs[i][j - 1])
            };
        }
    }
    let mut matches = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if same(i, j) && lcs[i][j] == lcs[i - 1][j - 1] + 1 {
            matches.push((i, j));
            (i, j) = (i - 1, j - 1);
        } else if lcs[i - 1][j] >= lcs[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    matches.reverse();
    matches
}

/// A script word as a recognizer would report it: a number below a thousand
/// in words, since recognizers spell numbers out; anything else as it is.
fn spoken(word: &str) -> Vec<String> {
    match word.parse::<u32>() {
        Ok(n) if n < 1000 => number_words(n).split(' ').map(str::to_string).collect(),
        _ => vec![word.to_string()],
    }
}

fn number_words(n: u32) -> String {
    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    match n {
        0..=19 => ONES[n as usize].to_string(),
        20..=99 if n % 10 == 0 => TENS[(n / 10) as usize].to_string(),
        20..=99 => format!("{} {}", TENS[(n / 10) as usize], ONES[(n % 10) as usize]),
        _ if n % 100 == 0 => format!("{} hundred", ONES[(n / 100) as usize]),
        _ => format!(
            "{} hundred {}",
            ONES[(n / 100) as usize],
            number_words(n % 100)
        ),
    }
}

/// A written word as a recognizer reports it: lowercase, split at anything
/// but letters, digits and apostrophes, so `command-line` is two words.
fn parts(word: &str) -> impl Iterator<Item = String> + '_ {
    word.split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|p| !p.is_empty())
        .map(str::to_lowercase)
}
