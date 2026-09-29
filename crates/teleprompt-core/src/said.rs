//! A line reworded to what its take says: where the recognizer heard other
//! words than the script's, the line as it was said, to keep instead of
//! reading it again.

use std::ops::Range;

/// `text` as `heard` says it, or `None` where it says the same words (or
/// nothing). The script's own words keep their capitals and punctuation;
/// a word heard instead of one takes its punctuation. A word misheard, a
/// letter or two off or heard as two, is the script's word.
pub fn reworded(text: &str, heard: &str) -> Option<String> {
    let script: Vec<&str> = text.split_whitespace().collect();
    let said = words(heard);
    let written: Vec<String> = script.iter().map(|w| norm(w)).collect();
    let steps = align(&written, &said);
    if said.is_empty() || steps.iter().all(|s| matches!(s, Step::Keep(..))) {
        return None;
    }
    // Each word out, and whether the script began a sentence with it.
    let mut out: Vec<(String, bool)> = Vec::new();
    // A dropped word's stop, for the word before it or the one said instead.
    let mut carry = String::new();
    // Only a sentence's end carries, in place of the word's own stop.
    let flush = |out: &mut Vec<(String, bool)>, carry: &mut String| {
        if let (Some((last, _)), Some(end)) = (out.last_mut(), carry.rfind(['.', '!', '?'])) {
            if !ends_sentence(last) {
                let own = last.len() - stop(last).len();
                last.truncate(own);
                last.push_str(&carry[end..]);
            }
        }
        carry.clear();
    };
    for step in steps {
        match step {
            Step::Keep(kept, _) => {
                flush(&mut out, &mut carry);
                for i in kept {
                    let first = i == 0 || ends_sentence(script[i - 1]);
                    out.push((script[i].to_string(), first));
                }
            }
            Step::Drop(i) => carry.push_str(stop(script[i])),
            Step::Add(j) => out.push((said[j].clone(), false)),
        }
    }
    flush(&mut out, &mut carry);
    let mut words = Vec::with_capacity(out.len());
    for (k, (word, began)) in out.into_iter().enumerate() {
        let starts = k == 0 || words.last().is_some_and(|w: &String| ends_sentence(w));
        words.push(if starts || is_i(&word) {
            capitalised(&word)
        } else if began {
            lowered(&word)
        } else {
            word
        });
    }
    Some(words.join(" "))
}

/// A take's transcript split into the `lines` it read: each word heard to
/// the line of the script's word it was; one heard between two lines, to
/// the line after.
pub fn per_line(lines: &[&str], heard: &str) -> Vec<String> {
    let mut written = Vec::new();
    let mut line_of = Vec::new();
    for (l, text) in lines.iter().enumerate() {
        for w in text.split_whitespace() {
            written.push(norm(w));
            line_of.push(l);
        }
    }
    let said = words(heard);
    let mut out = vec![Vec::new(); lines.len()];
    let mut waiting: Vec<usize> = Vec::new();
    let mut last = 0;
    for step in align(&written, &said) {
        match step {
            Step::Keep(i, j) => {
                last = line_of[i.start];
                out[last].extend(waiting.drain(..).chain(j).map(|j| said[j].clone()));
            }
            Step::Add(j) => waiting.push(j),
            Step::Drop(_) => {}
        }
    }
    if let Some(words) = out.get_mut(last) {
        words.extend(waiting.into_iter().map(|j| said[j].clone()));
    }
    out.into_iter().map(|w| w.join(" ")).collect()
}

/// A run of words in a line against what its take says, in reading order.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "words", rename_all = "lowercase")]
pub enum Change {
    Same(String),
    /// In the line, not said.
    Gone(String),
    /// Said, not in the line.
    New(String),
}

/// `line` against `said`, word for word, by their longest common run of
/// words; in a gap, what goes before what comes. What the prompters show
/// before keeping what was said.
pub fn diff(line: &str, said: &str) -> Vec<Change> {
    let a: Vec<&str> = line.split_whitespace().collect();
    let b: Vec<&str> = said.split_whitespace().collect();
    let (n, m) = (a.len(), b.len());
    let mut common = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            common[i][j] = if a[i] == b[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let mut out: Vec<Change> = Vec::new();
    let mut push = |change: Change| match (out.last_mut(), &change) {
        (Some(Change::Same(run)), Change::Same(w))
        | (Some(Change::Gone(run)), Change::Gone(w))
        | (Some(Change::New(run)), Change::New(w)) => {
            run.push(' ');
            run.push_str(w);
        }
        _ => out.push(change),
    };
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            push(Change::Same(a[i].into()));
            (i, j) = (i + 1, j + 1);
        } else if i < n && (j == m || common[i + 1][j] >= common[i][j + 1]) {
            push(Change::Gone(a[i].into()));
            i += 1;
        } else {
            push(Change::New(b[j].into()));
            j += 1;
        }
    }
    out
}

fn words(heard: &str) -> Vec<String> {
    heard
        .split_whitespace()
        .map(norm)
        .filter(|w| !w.is_empty())
        .collect()
}

/// Script words against words heard, in reading order: kept (the script's
/// words and the heard ones that are them), dropped, or added.
enum Step {
    Keep(Range<usize>, Range<usize>),
    Drop(usize),
    Add(usize),
}

/// The alignment that keeps the most of the script's words: a word heard
/// as itself, or else misheard, split in two or run together with the
/// next; in a gap, the dropped words before the ones said in their place.
fn align(written: &[String], said: &[String]) -> Vec<Step> {
    let (n, m) = (written.len(), said.len());
    // The ways script words from `i` and heard words from `j` can be kept,
    // and what each is worth: a word heard as itself, more than misheard.
    let pairs = |i: usize, j: usize| {
        let mut ways = Vec::new();
        if i < n && j < m && written[i] == said[j] {
            ways.push((1, 1, 3));
        } else if i < n && j < m && alike(&written[i], &said[j]) {
            ways.push((1, 1, 2));
        }
        // Split or run together, by a letter at most in four.
        if i < n && j + 1 < m && off(&written[i], &format!("{}{}", said[j], said[j + 1]), 4) {
            ways.push((1, 2, 2));
        }
        if i + 1 < n && j < m && off(&format!("{}{}", written[i], written[i + 1]), &said[j], 4) {
            ways.push((2, 1, 3));
        }
        ways
    };
    let mut best = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..=n).rev() {
        for j in (0..=m).rev() {
            let mut b = 0;
            for (a, c, worth) in pairs(i, j) {
                b = b.max(worth + best[i + a][j + c]);
            }
            if i < n {
                b = b.max(best[i + 1][j]);
            }
            if j < m {
                b = b.max(best[i][j + 1]);
            }
            best[i][j] = b;
        }
    }
    let (mut i, mut j, mut steps) = (0, 0, Vec::new());
    while i < n || j < m {
        if let Some((a, c, _)) = pairs(i, j)
            .into_iter()
            .find(|&(a, c, worth)| worth + best[i + a][j + c] == best[i][j])
        {
            steps.push(Step::Keep(i..i + a, j..j + c));
            (i, j) = (i + a, j + c);
        } else if i < n && best[i + 1][j] == best[i][j] {
            steps.push(Step::Drop(i));
            i += 1;
        } else {
            steps.push(Step::Add(j));
            j += 1;
        }
    }
    steps
}

/// The same word, or near enough to be it misheard: a letter off in three.
fn alike(a: &str, b: &str) -> bool {
    off(a, b, 3)
}

/// Whether `a` and `b` are a letter apart in `per` at most, apostrophes
/// aside; a word of one letter is only itself.
fn off(a: &str, b: &str, per: usize) -> bool {
    if a == b {
        return true;
    }
    let a: Vec<char> = a.chars().filter(|c| *c != '\'').collect();
    let b: Vec<char> = b.chars().filter(|c| *c != '\'').collect();
    if a.len().min(b.len()) < 2 {
        return a == b;
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (x, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = x + 1;
        for (y, cb) in b.iter().enumerate() {
            let next = (diagonal + usize::from(ca != cb))
                .min(row[y] + 1)
                .min(row[y + 1] + 1);
            diagonal = row[y + 1];
            row[y + 1] = next;
        }
    }
    row[b.len()] * per <= a.len().max(b.len())
}

/// A word as the recognizer would say it: lower case, letters, digits and
/// apostrophes.
fn norm(word: &str) -> String {
    word.chars()
        .map(|c| if c == '’' { '\'' } else { c })
        .filter(|c| c.is_alphanumeric() || *c == '\'')
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .trim_matches('\'')
        .to_string()
}

/// What follows a word's last letter or digit: its punctuation.
fn stop(word: &str) -> &str {
    let end = word
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_alphanumeric())
        .map_or(0, |(i, c)| i + c.len_utf8());
    &word[end..]
}

fn ends_sentence(word: &str) -> bool {
    stop(word).contains(['.', '!', '?'])
}

fn is_i(word: &str) -> bool {
    let w = norm(word);
    w == "i" || w.starts_with("i'")
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map_or(String::new(), |c| c.to_uppercase().chain(chars).collect())
}

fn lowered(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map_or(String::new(), |c| c.to_lowercase().chain(chars).collect())
}
