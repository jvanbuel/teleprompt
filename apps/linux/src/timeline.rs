//! The timeline strip: the script's lines and shots in time, as `teleprompt
//! plan --format json` lays them out, and what a drag onto it means as an
//! edit `teleprompt edit` makes.
//!
//! Only shots move or stretch. A line is the voice's length, and a take's:
//! nothing here offers to change one.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    pub duration_ms: u64,
    pub lines: Vec<LineSpan>,
    pub shots: Vec<ShotSpan>,
}

/// A narration line, from its first to its last sound.
#[derive(Debug, Clone, PartialEq)]
pub struct LineSpan {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Spoken from a take, rather than synthesized.
    pub recorded: bool,
}

/// A shot on screen.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotSpan {
    /// As `plan` names it: `<block>#<n>`.
    pub shot: String,
    pub scene: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// The line it runs with or after, if it has one.
    pub line: Option<String>,
    /// Whether it states its own length, and so can be stretched; one that
    /// does not takes its line's.
    pub timed: bool,
}

impl ShotSpan {
    /// The block it is a shot of, which is what an edit moves.
    pub fn block(&self) -> &str {
        self.shot.split_once('#').map_or(&self.shot, |(b, _)| b)
    }

    /// Only a block's first shot carries it: the others follow it.
    pub fn leads(&self) -> bool {
        self.shot.ends_with("#0")
    }
}

/// `plan --format json`, read as spans.
pub fn parse(json: &str) -> Result<Timeline, String> {
    let plan: Value =
        serde_json::from_str(json).map_err(|e| format!("cannot read the plan: {e}"))?;
    let ms = |v: &Value, key: &str| v[key].as_u64().unwrap_or(0);
    let mut timeline = Timeline {
        duration_ms: ms(&plan, "duration_ms"),
        lines: Vec::new(),
        shots: Vec::new(),
    };
    for entry in plan["entries"].as_array().into_iter().flatten() {
        let narration = &entry["narration"];
        let line = narration["line"].as_str().map(str::to_string);
        if let Some(id) = &line {
            let start = ms(narration, "start_ms");
            timeline.lines.push(LineSpan {
                id: id.clone(),
                start_ms: start,
                end_ms: start + ms(narration, "duration_ms"),
                recorded: narration["recorded"].as_bool().unwrap_or(false),
            });
        }
        let action = &entry["action"];
        if let Some(shot) = action["shot"].as_str() {
            let start = ms(action, "start_ms");
            timeline.shots.push(ShotSpan {
                shot: shot.to_string(),
                scene: action["scene"].as_str().unwrap_or("").to_string(),
                start_ms: start,
                end_ms: start + ms(action, "duration_ms"),
                line,
                timed: action["duration_source"].as_str() != Some("unknown"),
            });
        }
    }
    Ok(timeline)
}

/// Asks `binary` for `script`'s plan. Blocks while it compiles, so call it
/// off the main thread.
pub fn plan(binary: &Path, script: &Path) -> Result<Timeline, String> {
    let out = Command::new(binary)
        .args(["--format", "json", "plan"])
        .arg(script)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", binary.display()))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// What a drag asks of the script, as `teleprompt edit`'s arguments.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Start the block on its line's `word`th word.
    Cue { block: String, word: usize },
    /// Run the block after its line.
    Hold { block: String },
    /// Put the block with another line.
    Move {
        block: String,
        after: String,
        word: Option<usize>,
    },
    /// Make the block's shots `by` times as long.
    Stretch { block: String, by: f64 },
}

impl Edit {
    /// `teleprompt edit <script>`'s arguments after the script.
    pub fn args(&self) -> Vec<String> {
        match self {
            Edit::Cue { block, word } => {
                vec![
                    "cue".into(),
                    block.clone(),
                    "--word".into(),
                    word.to_string(),
                ]
            }
            Edit::Hold { block } => vec!["hold".into(), block.clone()],
            Edit::Move { block, after, word } => {
                let mut a = vec![
                    "move".into(),
                    block.clone(),
                    "--after".into(),
                    after.clone(),
                ];
                if let Some(w) = word {
                    a.extend(["--word".into(), w.to_string()]);
                }
                a
            }
            Edit::Stretch { block, by } => {
                vec![
                    "stretch".into(),
                    block.clone(),
                    "--by".into(),
                    format!("{by:.3}"),
                ]
            }
        }
    }
}

/// A shot's end dragged to `end_ms`: the stretch that makes it that long.
/// `None` for a shot that states no length, or a drag that changes nothing.
pub fn stretched(shot: &ShotSpan, end_ms: u64) -> Option<Edit> {
    if !shot.timed {
        return None;
    }
    let now = shot.end_ms.saturating_sub(shot.start_ms);
    let then = end_ms.saturating_sub(shot.start_ms);
    if now == 0 || then == 0 || now.abs_diff(then) < 50 {
        return None;
    }
    Some(Edit::Stretch {
        block: shot.block().to_string(),
        by: then as f64 / now as f64,
    })
}
