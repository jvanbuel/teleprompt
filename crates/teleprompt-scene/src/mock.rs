//! The M0 stand-in scene: a body of `wait` and `mark` lines.
//!
//! It exists so the compiler can produce a real timeline, `plan`, and `diff`
//! before any external runtime is involved, and it is the reference for what
//! a `SceneCompiler` has to answer.

// Rust guideline compliant 2026-07-18

use teleprompt_core::attrs::parse_duration_ms;
use teleprompt_core::{Diagnostic, Hash};

use crate::contract::{
    validate_commands, BlockSource, CommandError, Measured, SceneCompiler, Shot, Validated,
};

/// The mock adapter.
#[derive(Debug)]
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
#[derive(Debug)]
enum Command {
    /// Blank or a `#` comment: no executable content whatsoever. Ruling F13
    /// was written to keep these from becoming phantom shots; its wording
    /// said "whitespace-only" when it meant "no executable content", which
    /// is why a chunk of pure comments between two `mark`s still survived
    /// the filter as a zero-duration shot.
    Nothing,
    Mark,
    Wait(u64),
}

/// The lone place a mock body line is interpreted.
fn classify(line: &str) -> Result<Command, CommandError> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(Command::Nothing);
    }

    // `split_whitespace`, so a tab between a directive and its argument is
    // just whitespace — as `validate` always treated it, and as `estimate`
    // did not.
    let mut parts = line.split_whitespace();
    let head = parts.next().expect("a trimmed non-empty line has a token");

    match head {
        "mark" => match parts.next() {
            None => Ok(Command::Mark),
            Some(extra) => Err(CommandError::new(
                format!("`mark` takes no arguments, found `{extra}`"),
                "write `mark` on a line of its own",
            )),
        },
        "wait" => {
            let Some(value) = parts.next() else {
                return Err(CommandError::new(
                    "`wait` needs a duration",
                    "e.g. `wait 500ms`",
                ));
            };
            // Trailing garbage is rejected rather than ignored. It used to
            // pass `check` and then contribute nothing, which is the worst
            // of both: the author is told the script is fine and the clock
            // disagrees.
            if let Some(extra) = parts.next() {
                return Err(CommandError::new(
                    format!("`wait` takes one duration, found trailing `{extra}`"),
                    "e.g. `wait 500ms`",
                ));
            }
            match parse_duration_ms(value) {
                Ok(ms) => Ok(Command::Wait(ms)),
                Err(msg) => Err(CommandError::new(msg, "e.g. `wait 500ms`")),
            }
        }
        other => Err(CommandError::new(
            format!("unknown mock directive `{other}`"),
            "mock understands `wait <duration>` and `mark`",
        )),
    }
}

/// True when a mark-separated chunk carries something to execute. A chunk of
/// nothing but comments and blank lines is empty, not a zero-duration shot.
fn has_content(chunk: &str) -> bool {
    chunk
        .lines()
        .any(|l| !matches!(classify(l), Ok(Command::Nothing)))
}

impl SceneCompiler for MockScene {
    fn kind(&self) -> &'static str {
        "mock"
    }

    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        validate_commands(src, classify)
    }

    fn shots(&self, v: &Validated, block_id: &str) -> Result<Vec<Shot>, Vec<Diagnostic>> {
        let chunks: Vec<String> = v
            .body
            .lines()
            .collect::<Vec<_>>()
            .split(|l| matches!(classify(l), Ok(Command::Mark)))
            .map(|lines| lines.join("\n"))
            .filter(|chunk| has_content(chunk))
            .collect();

        Ok(chunks
            .into_iter()
            .enumerate()
            .map(|(index, source)| Shot {
                id: format!("{block_id}#{index}"),
                hash: Hash::of(source.trim().as_bytes()),
                source,
                index,
            })
            .collect())
    }

    fn estimate(&self, shot: &Shot) -> Measured {
        // Same `classify` as `validate` and `shots`, so a line that passed
        // `check` as a `wait` is a `wait` here too, tab or no tab.
        let total: u64 = shot
            .source
            .lines()
            .filter_map(|l| match classify(l) {
                Ok(Command::Wait(ms)) => Some(ms),
                // Not a catch-all: a directive added later must fail to
                // compile here rather than silently contribute nothing.
                Ok(Command::Nothing | Command::Mark) | Err(_) => None,
            })
            .sum();
        Measured::Exact(total)
    }
}
