//! A motion scene: JSX, rendered by Remotion.
//!
//! The block *is* JSX — the children of a full-frame composition, written
//! with whatever components the project's Remotion code exports. There is
//! no dialect here: what an author writes is what Remotion renders, and
//! everything Remotion can draw is available without this crate knowing
//! about it.
//!
//! What makes Remotion different from the other adapters is who decides
//! the length. A tape states its own timing; a Playwright script takes as
//! long as the page takes. A composition takes as long as it is *told* to:
//! `durationInFrames` is an argument, and a component that animates
//! against `useVideoConfig().durationInFrames` fills whatever it is given.
//! So `estimate` is [`Measured::Unknown`] — the block states no length —
//! and `retime` always succeeds, because rendering at the scheduled length
//! is not an approximation of the slot but the definition of it.

use teleprompt_core::{Diagnostic, Hash};
use teleprompt_scene::contract::{
    validate_commands, BlockSource, CommandError, Measured, SceneCompiler, Shot, Validated,
};

/// The mark, spelled as a JSX comment.
///
/// A comment for the reason every adapter's mark is one: the block has to
/// stay something a person can paste into a Remotion composition
/// unchanged. In JSX that means the braced form — a `//` line inside
/// children is not a comment but text, and would be drawn on screen.
pub const MARK: &str = "{/* mark */}";

/// How a re-timed shot states its length: a JSX comment, so the source
/// stays renderable, carrying the number that makes its hash move.
const LENGTH_PREFIX: &str = "{/* teleprompt: ";
const LENGTH_SUFFIX: &str = "ms */}";

#[derive(Debug, Default, Clone, Copy)]
pub struct RemotionScene;

impl SceneCompiler for RemotionScene {
    fn kind(&self) -> &'static str {
        "remotion"
    }

    /// Refuses one thing: a mark spelled the way the Playwright adapter
    /// spells it.
    ///
    /// Everything else is JSX this crate does not parse, and a wrong line
    /// fails when Remotion bundles it, with the bundler's own error. But
    /// `// mark` is worse than wrong: it is valid JSX that means the text
    /// "// mark", so the block would compile as one shot and render the
    /// author's intended split as a caption.
    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        validate_commands(src, |line| {
            if line.trim() == "// mark" {
                Err(CommandError::new(
                    "`// mark` is text in JSX, not a comment, and would be drawn on screen",
                    format!("a remotion block marks a shot with `{MARK}`"),
                ))
            } else {
                Ok(())
            }
        })
    }

    fn shots(&self, v: &Validated, block_id: &str) -> Result<Vec<Shot>, Vec<Diagnostic>> {
        let mut out = Vec::new();
        for chunk in v.body.split('\n').fold(vec![Vec::new()], |mut acc, line| {
            if line.trim() == MARK {
                acc.push(Vec::new());
            } else {
                acc.last_mut().expect("seeded with one").push(line);
            }
            acc
        }) {
            let source = chunk.join("\n");
            // A chunk of only comments and blank lines draws nothing, and
            // a shot that draws nothing is a picture nobody asked for.
            if !has_markup(&source) {
                continue;
            }
            let index = out.len();
            out.push(Shot {
                id: format!("{block_id}#{index}"),
                hash: Hash::of(source.as_bytes()),
                source,
                index,
            });
        }
        Ok(out)
    }

    /// A composition is as long as it is told to be, so the block itself
    /// says nothing about it. The scheduler gives the shot the sentence
    /// spoken over it.
    fn estimate(&self, _shot: &Shot) -> Measured {
        Measured::Unknown
    }

    /// A composition draws the same frames whatever was on screen before
    /// it, so a shot is named by its own source and editing one re-renders
    /// that one.
    fn continues(&self) -> bool {
        false
    }

    /// The same JSX, stating the length it will be rendered at.
    ///
    /// Nothing about the markup changes — Remotion is told the length, the
    /// markup is not — but the length has to be *in the source*, because
    /// the source is what the capture key is built from. A composition
    /// that animates across its whole duration is a different picture at
    /// four seconds than at six, and without this line a reworded sentence
    /// would reuse the clip rendered for the old one.
    fn retime(&self, shot: &Shot, target_ms: u64) -> Option<String> {
        Some(format!(
            "{LENGTH_PREFIX}{target_ms}{LENGTH_SUFFIX}\n{}",
            strip_length(&shot.source)
        ))
    }
}

/// A source without the length line a previous `retime` put on it, so
/// re-timing twice states one length rather than two.
fn strip_length(source: &str) -> &str {
    match source.split_once('\n') {
        Some((first, rest))
            if first.starts_with(LENGTH_PREFIX) && first.ends_with(LENGTH_SUFFIX) =>
        {
            rest
        }
        _ => source,
    }
}

/// Whether a chunk holds anything that would be drawn.
fn has_markup(source: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .any(|l| !l.is_empty() && !is_comment(l))
}

/// Whether a trimmed line is exactly one JSX comment, `{/* … */}`.
fn is_comment(line: &str) -> bool {
    line.len() >= 6
        && line.starts_with("{/*")
        && line.ends_with("*/}")
        && !line[3..line.len() - 3].contains("*/")
}
