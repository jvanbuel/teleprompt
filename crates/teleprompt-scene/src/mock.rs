use teleprompt_core::attrs::parse_duration_ms;
use teleprompt_core::{Diagnostic, Hash};

use crate::contract::{BlockSource, Measured, SceneCompiler, Span, Validated};

pub struct MockScene;

/// What one line of a mock body says.
///
/// This is the adapter's *single* notion of content, and all three trait
/// methods go through it. They used not to: `validate` split on whitespace
/// while `estimate` used `strip_prefix("wait ")`, so the two disagreed about
/// what a directive even was. A `check`-clean script could silently lose
/// five seconds:
///
/// ```text
/// wait<TAB>5000ms            -> check: ok -> contributed 0 ms
/// wait 5000ms and then some  -> check: ok -> contributed 0 ms
/// ```
///
/// The mock is M0's only source of action duration, so a disagreement here
/// is a disagreement about how long the video is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// Blank or a `#` comment: no executable content whatsoever. Ruling F13
    /// was written to keep these from becoming phantom beats; its wording
    /// said "whitespace-only" when it meant "no executable content", which
    /// is why a chunk of pure comments between two `mark`s still survived
    /// the filter as a zero-duration span.
    Nothing,
    Mark,
    Wait(u64),
}

/// The lone place a mock body line is interpreted.
///
/// `Err` carries the message and optional help for a diagnostic; the caller
/// supplies the location, since only it knows which file the line is in.
fn classify(line: &str) -> Result<Line, (String, Option<&'static str>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(Line::Nothing);
    }

    // `split_whitespace`, so a tab between a directive and its argument is
    // just whitespace — as `validate` always treated it, and as `estimate`
    // did not.
    let mut parts = line.split_whitespace();
    let head = parts.next().expect("a trimmed non-empty line has a token");

    match head {
        "mark" => match parts.next() {
            None => Ok(Line::Mark),
            Some(extra) => Err((
                format!("`mark` takes no arguments, found `{extra}`"),
                Some("write `mark` on a line of its own"),
            )),
        },
        "wait" => {
            let Some(value) = parts.next() else {
                return Err((
                    "`wait` needs a duration".to_string(),
                    Some("e.g. `wait 500ms`"),
                ));
            };
            // Trailing garbage is rejected rather than ignored. It used to
            // pass `check` and then contribute nothing, which is the worst
            // of both: the author is told the script is fine and the clock
            // disagrees.
            if let Some(extra) = parts.next() {
                return Err((
                    format!("`wait` takes one duration, found trailing `{extra}`"),
                    Some("e.g. `wait 500ms`"),
                ));
            }
            match parse_duration_ms(value) {
                Ok(ms) => Ok(Line::Wait(ms)),
                Err(msg) => Err((msg, Some("e.g. `wait 500ms`"))),
            }
        }
        other => Err((
            format!("unknown mock directive `{other}`"),
            Some("mock understands `wait <duration>` and `mark`"),
        )),
    }
}

/// True when a mark-separated chunk carries something to execute. A chunk of
/// nothing but comments and blank lines is empty, not a zero-duration beat.
fn has_content(chunk: &str) -> bool {
    chunk
        .lines()
        .any(|l| !matches!(classify(l), Ok(Line::Nothing)))
}

impl SceneCompiler for MockScene {
    fn kind(&self) -> &'static str {
        "mock"
    }

    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        let mut diags = Vec::new();
        for (i, line) in src.body.lines().enumerate() {
            if let Err((message, help)) = classify(line) {
                // Every diagnostic about a body line goes through `locate`,
                // which knows whether that line is in the script or in an
                // `include`d file. Doing the offset arithmetic here is what
                // made included bodies report a nonexistent line in the
                // wrong file.
                let mut d = Diagnostic::error(message);
                if let Some(h) = help {
                    d = d.with_help(h);
                }
                diags.push(src.origin.locate(d, i, line.trim().len()));
            }
        }

        if diags.is_empty() {
            Ok(Validated {
                scene: src.scene.clone(),
                body: src.body.clone(),
            })
        } else {
            Err(diags)
        }
    }

    fn spans(&self, v: &Validated, block_id: &str) -> Result<Vec<Span>, Vec<Diagnostic>> {
        let chunks: Vec<String> = v
            .body
            .lines()
            .collect::<Vec<_>>()
            .split(|l| matches!(classify(l), Ok(Line::Mark)))
            .map(|lines| lines.join("\n"))
            .filter(|chunk| has_content(chunk))
            .collect();

        Ok(chunks
            .into_iter()
            .enumerate()
            .map(|(index, source)| Span {
                id: format!("{block_id}#{index}"),
                hash: Hash::of(source.trim().as_bytes()),
                source,
                index,
            })
            .collect())
    }

    fn estimate(&self, span: &Span) -> Measured {
        // Same `classify` as `validate` and `spans`, so a line that passed
        // `check` as a `wait` is a `wait` here too, tab or no tab.
        let total: u64 = span
            .source
            .lines()
            .filter_map(|l| match classify(l) {
                Ok(Line::Wait(ms)) => Some(ms),
                _ => None,
            })
            .sum();
        Measured::Exact(total)
    }
}
