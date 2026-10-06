//! A stand-in scene of `wait` and `mark` lines, for testing the compiler's
//! side of the contract without an external tool.

use teleprompt_core::attrs::parse_duration_ms;
use teleprompt_core::{BlockId, Diagnostic, Hash, ShotId};

use super::contract::{
    validate_commands, BlockSource, CommandError, Measured, SceneCompiler, Shot, Validated,
};

#[derive(Debug)]
pub struct MockScene;

/// What one mock body line says. `validate`, `shots` and `length` all read
/// lines through [`classify`], so a line that passes `check` counts towards
/// the duration.
#[derive(Debug)]
enum Command {
    /// Blank or a `#` comment. A chunk of only these is not a shot.
    Nothing,
    Mark,
    Wait(u64),
    /// A shot that states no length, as a Playwright script does.
    Open,
}

fn classify(line: &str) -> Result<Command, CommandError> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(Command::Nothing);
    }

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
        "open" => match parts.next() {
            None => Ok(Command::Open),
            Some(extra) => Err(CommandError::new(
                format!("`open` takes no arguments, found `{extra}`"),
                "write `open` on a line of its own",
            )),
        },
        "wait" => {
            let Some(value) = parts.next() else {
                return Err(CommandError::new(
                    "`wait` needs a duration",
                    "e.g. `wait 500ms`",
                ));
            };
            // Rejected, not ignored: `check` passing on a line that adds no
            // time would misstate the video's length.
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
            "mock understands `wait <duration>`, `open` and `mark`",
        )),
    }
}

fn has_content(chunk: &str) -> bool {
    chunk
        .lines()
        .any(|l| !matches!(classify(l), Ok(Command::Nothing)))
}

/// What the waits add up to, exactly; `Unknown` once a shot opens
/// something, which takes as long as it takes.
pub fn length(source: &str) -> Measured {
    if source
        .lines()
        .any(|l| matches!(classify(l), Ok(Command::Open)))
    {
        return Measured::Unknown;
    }
    let total: u64 = source
        .lines()
        .filter_map(|l| match classify(l) {
            Ok(Command::Wait(ms)) => Some(ms),
            // No catch-all, so a new directive must be handled here.
            Ok(Command::Nothing | Command::Mark | Command::Open) | Err(_) => None,
        })
        .sum();
    Measured::Exact(total)
}

impl SceneCompiler for MockScene {
    fn kind(&self) -> &'static str {
        "mock"
    }

    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        validate_commands(src, classify)
    }

    fn shots(&self, v: &Validated, block_id: &BlockId) -> Result<Vec<Shot>, Vec<Diagnostic>> {
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
                id: ShotId::of(block_id, index),
                hash: Hash::of(source.trim().as_bytes()),
                length: length(&source),
                source,
                index,
            })
            .collect())
    }
}
