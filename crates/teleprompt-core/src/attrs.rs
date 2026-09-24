use std::collections::BTreeMap;

use crate::{Diagnostic, DurationMs, SourceSpan};

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

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// A line's attributes, each value already parsed. One that does not parse
/// is a diagnostic at the line and `None` here, never a string for a later
/// step to drop.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LineAttrs {
    pub voice_source: Option<String>,
    pub voice_backend: Option<String>,
    pub voice: Option<String>,
    pub voice_speed: Option<f64>,
    pub lead_in: Option<DurationMs>,
    pub tail: Option<DurationMs>,
    pub lang: Option<String>,
}

impl LineAttrs {
    pub fn parse(raw: &str, span: SourceSpan) -> (Self, Vec<Diagnostic>) {
        let (a, mut diags) = parse_attrs(raw, SEGMENT_KEYS, span);
        let mut v = Values {
            a: &a,
            span,
            diags: &mut diags,
        };
        let attrs = Self {
            voice_source: v.text("voice.source"),
            voice_backend: v.text("voice.backend"),
            voice: v.text("voice.voice"),
            voice_speed: v.parsed("voice.speed", number),
            lead_in: v.parsed("lead_in", DurationMs::parse),
            tail: v.parsed("tail", DurationMs::parse),
            lang: v.text("lang"),
        };
        (attrs, diags)
    }
}

/// An action block's attributes, each value already parsed; see
/// [`LineAttrs`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlockAttrs {
    pub scene: Option<String>,
    pub include: Option<String>,
    pub review: Option<String>,
    pub policy: Option<String>,
    pub align: Option<String>,
    pub cue: Option<String>,
    pub session: Option<String>,
    pub id: Option<String>,
    pub max_speedup: Option<f64>,
    pub max_stretch: Option<f64>,
    pub min_stretch: Option<f64>,
}

impl BlockAttrs {
    pub fn parse(raw: &str, span: SourceSpan) -> (Self, Vec<Diagnostic>) {
        let (a, mut diags) = parse_attrs(raw, BLOCK_KEYS, span);
        let mut v = Values {
            a: &a,
            span,
            diags: &mut diags,
        };
        let attrs = Self {
            scene: v.text("scene"),
            include: v.text("include"),
            review: v.text("review"),
            policy: v.text("policy"),
            align: v.text("align"),
            cue: v.text("cue"),
            session: v.text("session"),
            id: v.text("id"),
            max_speedup: v.parsed("max_speedup", number),
            max_stretch: v.parsed("max_stretch", number),
            min_stretch: v.parsed("min_stretch", number),
        };
        (attrs, diags)
    }
}

/// Reads typed values out of parsed attributes, reporting each that fails.
struct Values<'a> {
    a: &'a Attributes,
    span: SourceSpan,
    diags: &'a mut Vec<Diagnostic>,
}

impl Values<'_> {
    fn text(&self, key: &str) -> Option<String> {
        self.a.get(key).map(str::to_string)
    }

    fn parsed<T>(&mut self, key: &str, parse: impl Fn(&str) -> Result<T, String>) -> Option<T> {
        match parse(self.a.get(key)?) {
            Ok(value) => Some(value),
            Err(why) => {
                let d = Diagnostic::error(format!("`{key}`: {why}")).at(self.span);
                self.diags.push(d);
                None
            }
        }
    }
}

fn number(v: &str) -> Result<f64, String> {
    v.parse::<f64>()
        .map_err(|_| format!("`{v}` is not a number"))
}

/// [`DurationMs::parse`] as milliseconds, for adapters that count in `u64`.
pub fn parse_duration_ms(v: &str) -> Result<u64, String> {
    DurationMs::parse(v).map(DurationMs::ms)
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
