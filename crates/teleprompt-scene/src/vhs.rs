//! The `terminal` scene, served by VHS tapes.
//!
//! The block body is real VHS tape syntax and stays runnable by `vhs` itself
//! — including the mark, which is spelled as a comment for exactly that
//! reason (§7.5). teleprompt reuses the language; the runtime is its own PTY.

// Rust guideline compliant 2026-07-18

use teleprompt_core::attrs::parse_duration_ms;
use teleprompt_core::{Diagnostic, Hash};

use crate::contract::{
    validate_lines, BlockSource, LineError, Measured, SceneCompiler, Span, Validated,
};

/// The VHS adapter.
#[derive(Debug)]
pub struct VhsScene;

/// VHS's own default gap between keystrokes, overridable per tape with
/// `Set TypingSpeed` or per command with `Type@100ms`. Matching the tool's
/// default is what makes an unannotated tape estimate correctly.
const DEFAULT_TYPING_SPEED_MS: u64 = 50;

/// Keys VHS presses. `Ctrl+`/`Alt+`/`Shift+` combinations are recognised by
/// prefix instead, since their tails are arbitrary.
const KEYS: &[&str] = &[
    "Enter",
    "Tab",
    "Space",
    "Backspace",
    "Delete",
    "Escape",
    "Up",
    "Down",
    "Left",
    "Right",
    "PageUp",
    "PageDown",
];

/// Commands that change the tape's state but consume no wall clock.
const INSTANT: &[&str] = &[
    "Require",
    "Hide",
    "Show",
    "Screenshot",
    "Env",
    "Source",
    "Copy",
    "Paste",
];

/// The mark, spelled as a VHS comment.
///
/// VHS has no yield-point concept, and adding a `Mark` command would make the
/// body a teleprompt dialect rather than a tape — forfeiting the only reason
/// to point `include=` at a real `.tape` file, which is that `vhs demo.tape`
/// runs it and VHS's own tooling understands it. As a comment it is inert.
const MARK: &str = "# mark";

/// What one tape line contributes.
///
/// This is the adapter's single notion of content, and all three trait
/// methods go through it — the same discipline `MockScene` documents. A
/// `Type@100ms` that `validate` accepts and `estimate` does not understand
/// is not a parse bug, it is a video that is the wrong length while `check`
/// reports success.
#[derive(Debug)]
enum Line {
    /// Blank, a comment, or a command with no duration.
    Nothing,
    Mark,
    Sleep(u64),
    /// `speed` is a `Type@<duration>` override; `None` means "whatever
    /// `Set TypingSpeed` last established".
    Type {
        chars: u64,
        speed: Option<u64>,
    },
    Keys(u64),
    TypingSpeed(u64),
}

/// The lone place a tape line is interpreted.
fn classify(line: &str) -> Result<Line, LineError> {
    let line = line.trim();

    if line == MARK {
        return Ok(Line::Mark);
    }
    if line.is_empty() || line.starts_with('#') {
        return Ok(Line::Nothing);
    }

    let mut parts = line.split_whitespace();
    let head = parts.next().expect("a trimmed non-empty line has a token");

    // `Type@100ms "hello"` — VHS hangs the per-command speed off the name.
    let (cmd, at) = match head.split_once('@') {
        Some((cmd, dur)) => {
            let ms = parse_duration_ms(dur).map_err(|m| LineError::new(m, "e.g. `Type@100ms`"))?;
            (cmd, Some(ms))
        }
        None => (head, None),
    };

    match cmd {
        "Sleep" => {
            let Some(value) = parts.next() else {
                return Err(LineError::new(
                    "`Sleep` needs a duration",
                    "e.g. `Sleep 2s`",
                ));
            };
            // Trailing garbage is rejected rather than ignored, for the
            // reason `MockScene` gives: a line that passes `check` and then
            // contributes nothing is the worst of both outcomes.
            if let Some(extra) = parts.next() {
                return Err(LineError::new(
                    format!("`Sleep` takes one duration, found trailing `{extra}`"),
                    "e.g. `Sleep 2s`",
                ));
            }
            parse_duration_ms(value)
                .map(Line::Sleep)
                .map_err(|m| LineError::new(m, "e.g. `Sleep 2s`"))
        }

        "Type" => {
            let rest = line[head.len()..].trim();
            let quote = rest
                .chars()
                .next()
                .filter(|c| matches!(c, '"' | '\'' | '`'));
            let text = quote
                .and_then(|q| rest.strip_prefix(q)?.strip_suffix(q))
                .ok_or_else(|| {
                    LineError::new(
                        "`Type` needs a quoted string",
                        "e.g. `Type \"npm install\"`",
                    )
                })?;
            // A tape long enough to overflow this could not fit in memory,
            // but `as` would wrap silently if one ever did.
            let chars = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
            Ok(Line::Type { chars, speed: at })
        }

        "Set" => match (parts.next(), parts.next()) {
            (Some("TypingSpeed"), Some(value)) => parse_duration_ms(value)
                .map(Line::TypingSpeed)
                .map_err(|m| LineError::new(m, "e.g. `Set TypingSpeed 40ms`")),
            // teleprompt executes the tape against its own PTY (§7.5), so a
            // tape that picks its own shell picks one teleprompt is not
            // driving.
            (Some("Shell"), _) => Err(LineError::new(
                "`Set Shell` is set by teleprompt, not by the tape",
                "remove the line; configure the shell under `scene.terminal`",
            )),
            // FontSize, Theme, Width, Padding: real settings, no clock cost.
            (Some(_), Some(_)) => Ok(Line::Nothing),
            (Some(name), None) => Err(LineError::new(
                format!("`Set {name}` needs a value"),
                "e.g. `Set FontSize 32`",
            )),
            (None, _) => Err(LineError::new(
                "`Set` needs a name and a value",
                "e.g. `Set FontSize 32`",
            )),
        },

        // teleprompt owns framing and encoding: a tape that writes its own
        // file would produce a second, unscheduled artifact beside the one
        // the timeline expects.
        "Output" => Err(LineError::new(
            "`Output` is set by teleprompt, not by the tape",
            "remove the line; the compiler routes frames to the timeline",
        )),

        // `Wait` blocks on the shell prompt, so its real duration is however
        // long the command underneath takes — which is not in the tape, and
        // is why `estimate` returns `Estimated` rather than `Exact`.
        "Wait" => Ok(Line::Nothing),

        key if KEYS.contains(&key)
            || key.starts_with("Ctrl+")
            || key.starts_with("Alt+")
            || key.starts_with("Shift+") =>
        {
            let count = match parts.next() {
                None => 1,
                Some(value) => value.parse::<u64>().map_err(|_| {
                    LineError::new(
                        format!("`{key}` takes an optional repeat count, found `{value}`"),
                        "e.g. `Enter 3`",
                    )
                })?,
            };
            Ok(Line::Keys(count))
        }

        instant if INSTANT.contains(&instant) => Ok(Line::Nothing),

        other => Err(LineError::new(
            format!("unknown VHS command `{other}`"),
            "see the VHS tape reference for the command list",
        )),
    }
}

/// True when a mark-separated chunk carries something to execute.
///
/// A chunk of nothing but comments and `Set` lines is empty, not a
/// zero-duration beat (ruling F13). `TypingSpeed` counts as a setting here
/// even though it carries a duration: it configures later typing rather than
/// spending any time itself, and a chunk holding only settings is exactly the
/// phantom beat F13 rules out.
fn has_content(chunk: &[&str]) -> bool {
    chunk
        .iter()
        .any(|l| !matches!(classify(l), Ok(Line::Nothing | Line::TypingSpeed(_))))
}

impl SceneCompiler for VhsScene {
    fn kind(&self) -> &'static str {
        "vhs"
    }

    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        validate_lines(src, classify)
    }

    fn spans(&self, v: &Validated, block_id: &str) -> Result<Vec<Span>, Vec<Diagnostic>> {
        let lines: Vec<&str> = v.body.lines().collect();
        let mut settings: Vec<&str> = Vec::new();
        let mut spans: Vec<Span> = Vec::new();

        for chunk in lines.split(|l| matches!(classify(l), Ok(Line::Mark))) {
            // `Set` lines established before a mark still govern the tape
            // after it, so each span carries the settings in force when it
            // starts. That keeps `estimate` a pure function of `span.source`,
            // and puts the settings in the hash — so raising TypingSpeed
            // invalidates exactly the later spans whose timing it changes.
            let source = settings
                .iter()
                .chain(chunk.iter())
                .copied()
                .collect::<Vec<_>>()
                .join("\n");

            settings.extend(
                chunk
                    .iter()
                    .filter(|l| l.trim_start().starts_with("Set "))
                    .copied(),
            );

            if !has_content(chunk) {
                continue;
            }

            let index = spans.len();
            spans.push(Span {
                id: format!("{block_id}#{index}"),
                hash: Hash::of(source.trim().as_bytes()),
                source,
                index,
            });
        }

        Ok(spans)
    }

    fn estimate(&self, span: &Span) -> Measured {
        let mut speed = DEFAULT_TYPING_SPEED_MS;
        let mut total: u64 = 0;

        for line in span.source.lines() {
            match classify(line) {
                Ok(Line::Sleep(ms)) => total = total.saturating_add(ms),
                Ok(Line::TypingSpeed(ms)) => speed = ms,
                Ok(Line::Type { chars, speed: at }) => {
                    total = total.saturating_add(chars.saturating_mul(at.unwrap_or(speed)));
                }
                Ok(Line::Keys(n)) => total = total.saturating_add(n.saturating_mul(speed)),
                // Not a catch-all: a tape command added later must fail to
                // compile here rather than silently estimate as zero.
                Ok(Line::Nothing | Line::Mark) | Err(_) => {}
            }
        }

        // Not `Exact`. Every `Sleep` is exact, but a `Wait` sits on top of a
        // shell command whose runtime the tape does not state. M1's measuring
        // pass replaces this with an observed duration, cached by span hash.
        Measured::Estimated(total)
    }
}
