use std::collections::BTreeMap;

use crate::{Diagnostic, SourceSpan};

pub const SEGMENT_KEYS: &[&str] = &[
    "voice.source",
    "voice.backend",
    "voice.voice",
    "voice.speed",
    "lead_in",
    "tail",
    "lang",
];

pub const BLOCK_KEYS: &[&str] = &[
    "scene",
    "include",
    // Set by `from` on a tape it generated from someone else's document.
    // The command in it has been read by nobody, so `check` says so until a
    // human removes the attribute.
    "review",
    "policy",
    "align",
    // The phrase in the narration this action should start on.
    "cue",
    // Which run of the scene this block belongs to. Blocks naming the same
    // scene continue one session by default — that is what naming a scene
    // means, and it is why a walkthrough's elements show a running program
    // rather than six fresh shells. A `session=` names a different run, for
    // a script that quits something and starts it again.
    "session",
    "id",
    "max_speedup",
    "max_stretch",
    "min_stretch",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attributes(BTreeMap<String, String>);

impl Attributes {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn get_f64(&self, key: &str) -> Option<Result<f64, String>> {
        self.get(key).map(|v| {
            v.parse::<f64>()
                .map_err(|_| format!("`{v}` is not a number"))
        })
    }

    /// Parses `250ms`, `1s`, `1.5s`, or a bare integer treated as milliseconds.
    pub fn get_ms(&self, key: &str) -> Option<Result<u64, String>> {
        self.get(key).map(parse_duration_ms)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

pub fn parse_duration_ms(v: &str) -> Result<u64, String> {
    let err = || format!("`{v}` is not a duration (try `250ms` or `1s`)");
    if let Some(n) = v.strip_suffix("ms") {
        return n.trim().parse::<u64>().map_err(|_| err());
    }
    if let Some(n) = v.strip_suffix('s') {
        let secs: f64 = n.trim().parse().map_err(|_| err())?;
        if secs < 0.0 {
            return Err(err());
        }
        return Ok(crate::time::ms_from_seconds(secs));
    }
    v.parse::<u64>().map_err(|_| err())
}

pub fn parse_attrs(raw: &str, allowed: &[&str], span: SourceSpan) -> (Attributes, Vec<Diagnostic>) {
    let mut map = BTreeMap::new();
    let mut diags = Vec::new();

    for token in tokenize(raw) {
        if token.starts_with('#') {
            continue; // the id anchor, handled by ident.rs
        }
        let Some((key, value)) = token.split_once('=') else {
            diags
                .push(Diagnostic::error(format!("expected `key=value`, found `{token}`")).at(span));
            continue;
        };
        let key = key.trim();
        if !allowed.contains(&key) {
            let mut d = Diagnostic::error(format!("unknown attribute key `{key}`")).at(span);
            if let Some(sug) = nearest(key, allowed) {
                d = d.with_help(format!("did you mean `{sug}`?"));
            }
            diags.push(d);
            continue;
        }
        let value = value.trim().trim_matches('"').to_string();
        map.insert(key.to_string(), value);
    }

    (Attributes(map), diags)
}

/// Splits on whitespace, keeping double-quoted runs together.
fn tokenize(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for c in raw.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Levenshtein distance, returning the closest allowed key within distance 2.
fn nearest<'a>(key: &str, allowed: &[&'a str]) -> Option<&'a str> {
    allowed
        .iter()
        .map(|c| (*c, levenshtein(key, c)))
        .filter(|(_, d)| *d <= 2)
        .min_by_key(|(_, d)| *d)
        .map(|(c, _)| c)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut cur = vec![0usize; b_chars.len() + 1];

    for (i, ac) in a.chars().enumerate() {
        cur[0] = i + 1;
        for (j, bc) in b_chars.iter().enumerate() {
            let cost = usize::from(ac != *bc);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b_chars.len()]
}
