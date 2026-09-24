//! Drift detection for a committed narration manifest, behind
//! `dub --check` (docs/design.md#drift).

use serde::Serialize;
use teleprompt_core::time::short;

use crate::manifest::{LineEntry, NarrationManifest};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangedSegment {
    pub id: String,
    pub before_ms: u64,
    pub after_ms: u64,
    /// `text edited` | `now measured` | `audio changed` |
    /// `voice tier X → Y` | `voice request changed` | `shifted`. Different
    /// causes want different fixes, so the report must not collapse them.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestDiff {
    pub duration_before_ms: u64,
    pub duration_after_ms: u64,
    /// Drift even when every line is untouched: a consumer configures its
    /// player from `AudioInfo` once.
    pub audio_changed: bool,
    /// Its own flag because a retitle that slugifies identically changes no
    /// `LineEntry`.
    pub chapters_changed: bool,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<ChangedSegment>,
    pub reordered: bool,
}

impl ManifestDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && !self.reordered
            && !self.audio_changed
            && !self.chapters_changed
            && self.duration_before_ms == self.duration_after_ms
    }

    pub fn render(&self) -> String {
        if self.is_empty() {
            return "no narration changes\n".to_string();
        }

        let mut out = String::new();
        let delta = self.duration_after_ms as i64 - self.duration_before_ms as i64;
        out.push_str(&format!(
            "narration: {} → {} ({}{})\n",
            short(self.duration_before_ms),
            short(self.duration_after_ms),
            if delta >= 0 { "+" } else { "-" },
            short(delta.unsigned_abs()),
        ));

        if self.chapters_changed {
            out.push_str("\nchapters changed (titles or markers)\n");
        }
        if self.audio_changed {
            out.push_str("\naudio format changed (sample rate, channels, or format)\n");
        }

        if !self.added.is_empty() {
            out.push_str("\nadded:\n");
            for id in &self.added {
                out.push_str(&format!("  {id}\n"));
            }
        }
        if !self.removed.is_empty() {
            out.push_str("\nremoved:\n");
            for id in &self.removed {
                out.push_str(&format!("  {id}\n"));
            }
        }
        if !self.changed.is_empty() {
            out.push_str("\nchanged:\n");
            for c in &self.changed {
                out.push_str(&format!(
                    "  {:<16} {} → {}  ({})\n",
                    c.id,
                    short(c.before_ms),
                    short(c.after_ms),
                    c.reason
                ));
            }
        }
        if self.reordered {
            out.push_str("\nreordered: line order changed\n");
        }

        // A `shifted` line's audio is unchanged, so it needs no re-render.
        let mut stale: Vec<&str> = self
            .changed
            .iter()
            .filter(|c| c.reason != "shifted")
            .map(|c| c.id.as_str())
            .collect();
        stale.extend(self.added.iter().map(String::as_str));
        if !stale.is_empty() {
            out.push_str("\nneeds re-render:\n");
            for id in stale {
                out.push_str(&format!("  {id}\n"));
            }
        }

        out
    }
}

/// Why a line differs, naming the most actionable cause first: a text edit
/// explains everything downstream of it. `shifted` means only `start_ms`
/// moved, because an earlier line changed length.
///
/// The first three reasons are ordered as in the timeline's narration diff
/// (`teleprompt-schedule/src/diff.rs`); the two must not contradict.
fn reason_for(before: &LineEntry, after: &LineEntry) -> Option<String> {
    if before.source_hash != after.source_hash {
        Some("text edited".to_string())
    // Before the audio checks, which would otherwise absorb it. Only the
    // move to `measured` counts; with `null`, estimate and audio coincide,
    // so nothing else would report it.
    } else if before.duration_source != after.duration_source && after.duration_source == "measured"
    {
        Some("now measured".to_string())
    } else if before.audio_hash != after.audio_hash || before.duration_ms != after.duration_ms {
        Some("audio changed".to_string())
    } else if before.voice_source_actual != after.voice_source_actual {
        Some(format!(
            "voice tier {} → {}",
            before.voice_source_actual, after.voice_source_actual
        ))
    } else if before.voice_source != after.voice_source
        || before.downgrade_reason != after.downgrade_reason
    {
        Some("voice request changed".to_string())
    } else if before.start_ms != after.start_ms {
        Some("shifted".to_string())
    } else {
        None
    }
}

pub fn diff(before: &NarrationManifest, after: &NarrationManifest) -> ManifestDiff {
    let added = after
        .lines
        .iter()
        .filter(|s| !before.lines.iter().any(|b| b.id == s.id))
        .map(|s| s.id.clone())
        .collect();
    let removed = before
        .lines
        .iter()
        .filter(|s| !after.lines.iter().any(|a| a.id == s.id))
        .map(|s| s.id.clone())
        .collect();

    let changed = before
        .lines
        .iter()
        .filter_map(|b| {
            let a = after.lines.iter().find(|a| a.id == b.id)?;
            reason_for(b, a).map(|reason| ChangedSegment {
                id: b.id.clone(),
                before_ms: b.duration_ms,
                after_ms: a.duration_ms,
                reason,
            })
        })
        .collect();

    // Order is part of the contract: two lines that swapped are drift.
    let before_order: Vec<&str> = before.lines.iter().map(|s| s.id.as_str()).collect();
    let after_order: Vec<&str> = after.lines.iter().map(|s| s.id.as_str()).collect();
    let common_before: Vec<&str> = before_order
        .iter()
        .copied()
        .filter(|id| after_order.contains(id))
        .collect();
    let common_after: Vec<&str> = after_order
        .iter()
        .copied()
        .filter(|id| before_order.contains(id))
        .collect();

    ManifestDiff {
        duration_before_ms: before.duration_ms,
        duration_after_ms: after.duration_ms,
        audio_changed: before.audio != after.audio,
        chapters_changed: before.chapters != after.chapters,
        added,
        removed,
        changed,
        reordered: common_before != common_after,
    }
}
