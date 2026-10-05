use std::collections::BTreeMap;

use crate::policy::{Align, PolicyKind};
use crate::{Diagnostic, DurationMs, SourceSpan};

pub const SEGMENT_KEYS: &[&str] = &[
    "voice.backend",
    "voice.voice",
    "voice.speed",
    "voice.instruct",
    "lead_in",
    "tail",
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
    "trim_warn_above",
    "max_stretch",
    "min_stretch",
    // How much longer (above 1) or shorter its shots run than their own
    // pace: an author's own re-timing, where `fit-action` is the line's.
    "stretch",
    // The length of a `fit-line` item whose shot cannot state one.
    "budget",
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
    pub voice_backend: Option<String>,
    pub voice: Option<String>,
    pub voice_speed: Option<f64>,
    pub voice_instruct: Option<String>,
    pub lead_in: Option<DurationMs>,
    pub tail: Option<DurationMs>,
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
            voice_backend: v.text("voice.backend"),
            voice: v.text("voice.voice"),
            voice_speed: v.parsed("voice.speed", number),
            voice_instruct: v.text("voice.instruct"),
            lead_in: v.parsed("lead_in", DurationMs::parse),
            tail: v.parsed("tail", DurationMs::parse),
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
    pub policy: Option<PolicyKind>,
    pub align: Option<Align>,
    pub cue: Option<String>,
    pub session: Option<String>,
    pub id: Option<String>,
    pub budget: Option<DurationMs>,
    pub trim_warn_above: Option<f64>,
    pub max_stretch: Option<f64>,
    pub min_stretch: Option<f64>,
    pub stretch: Option<f64>,
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
            policy: v.explained("policy", PolicyKind::parse),
            align: v.explained("align", Align::parse),
            cue: v.text("cue"),
            session: v.text("session"),
            id: v.text("id"),
            budget: v.parsed("budget", DurationMs::parse),
            trim_warn_above: v.parsed("trim_warn_above", number),
            max_stretch: v.parsed("max_stretch", number),
            min_stretch: v.parsed("min_stretch", number),
            stretch: v.parsed("stretch", factor),
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

impl Values<'_> {
    /// Like [`Self::parsed`], for a parser that writes its own message and
    /// help.
    fn explained<T>(
        &mut self,
        key: &str,
        parse: impl Fn(&str) -> Result<T, (String, String)>,
    ) -> Option<T> {
        match parse(self.a.get(key)?) {
            Ok(value) => Some(value),
            Err((message, help)) => {
                let d = Diagnostic::error(message).at(self.span).with_help(help);
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

/// A factor: a number above zero, such as `1.5` or `0.8`.
fn factor(v: &str) -> Result<f64, String> {
    number(v).and_then(|f| {
        (f.is_finite() && f > 0.0)
            .then_some(f)
            .ok_or_else(|| format!("`{v}` is not a factor above zero, such as 1.5"))
    })
}

/// [`DurationMs::parse`] as milliseconds, for scene plugins that count in `u64`.
pub fn parse_duration_ms(v: &str) -> Result<u64, String> {
    DurationMs::parse(v).map(DurationMs::ms)
}

/// Whether `name` can name a speaker: a letter, then letters, digits, `-`
/// and `_`, as a TOML table key would be written bare.
pub fn is_speaker_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `@name` among attributes, which is not how a speaker is named.
fn at_name(token: &str, span: SourceSpan) -> Diagnostic {
    let name = &token[1..];
    let mut label = name.chars();
    let label: String = label
        .next()
        .map(|c| c.to_uppercase().chain(label).collect())
        .unwrap_or_default();
    Diagnostic::error(format!("`{token}` is not an attribute"))
        .at(span)
        .with_help(format!(
            "a speaker is named before what they say: start the line with `**{label}:**`"
        ))
}

/// A chapter heading's `{…}` settings, as `(key, value)` in the order
/// written: any setting the front matter takes, its path dotted, as in
/// `{speaker=guest voice.speed=1.1}`. The `#id` is the parser's.
pub fn heading_attrs(raw: &str, span: SourceSpan) -> (Vec<(String, String)>, Vec<Diagnostic>) {
    let mut out = Vec::new();
    let mut diags = Vec::new();
    for token in tokenize(raw) {
        if token.starts_with('#') {
            continue;
        }
        if token.starts_with('@') {
            diags.push(at_name(&token, span));
            continue;
        }
        match token.split_once('=') {
            Some((key, value)) if !key.trim().is_empty() => out.push((
                key.trim().to_string(),
                value.trim().trim_matches('"').to_string(),
            )),
            _ => diags
                .push(Diagnostic::error(format!("expected `key=value`, found `{token}`")).at(span)),
        }
    }
    (out, diags)
}

pub fn parse_attrs(raw: &str, allowed: &[&str], span: SourceSpan) -> (Attributes, Vec<Diagnostic>) {
    let mut map = BTreeMap::new();
    let mut diags = Vec::new();

    for token in tokenize(raw) {
        if token.starts_with('#') {
            continue; // the id anchor, handled by ident.rs
        }
        if token.starts_with('@') {
            diags.push(at_name(&token, span));
            continue;
        }
        let Some((key, value)) = token.split_once('=') else {
            diags
                .push(Diagnostic::error(format!("expected `key=value`, found `{token}`")).at(span));
            continue;
        };
        let key = key.trim();
        if key == "speaker" && allowed == SEGMENT_KEYS {
            diags.push(at_name(
                &format!("@{}", value.trim().trim_matches('"')),
                span,
            ));
            continue;
        }
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
