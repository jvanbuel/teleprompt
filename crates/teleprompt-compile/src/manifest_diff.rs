//! Drift detection for a committed narration manifest.
//!
//! This is what keeps teleprompt's central claim working when the picture
//! is rendered by something teleprompt does not control: a pull request
//! that edits prose without regenerating fails, exactly as one with a
//! stale timeline does.

use serde::Serialize;

use crate::manifest::{NarrationManifest, SegmentEntry};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangedSegment {
    pub id: String,
    pub before_ms: u64,
    pub after_ms: u64,
    /// `text edited` | `audio changed` | `moved`. Different causes want
    /// different fixes, so the report must not collapse them.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestDiff {
    pub duration_before_ms: u64,
    pub duration_after_ms: u64,
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
            secs(self.duration_before_ms),
            secs(self.duration_after_ms),
            if delta >= 0 { "+" } else { "-" },
            secs(delta.unsigned_abs()),
        ));

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
                    secs(c.before_ms),
                    secs(c.after_ms),
                    c.reason
                ));
            }
        }
        if self.reordered {
            out.push_str("\nreordered: segment order changed\n");
        }

        let mut stale: Vec<&str> = self.changed.iter().map(|c| c.id.as_str()).collect();
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

fn secs(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

/// Why a segment differs, in the order that makes the report most useful:
/// a text edit is the author's own doing and explains everything
/// downstream, so it is named even when the audio and position also moved.
fn reason_for(before: &SegmentEntry, after: &SegmentEntry) -> Option<String> {
    if before.source_hash != after.source_hash {
        Some("text edited".to_string())
    } else if before.audio_hash != after.audio_hash {
        Some("audio changed".to_string())
    } else if before.start_ms != after.start_ms || before.duration_ms != after.duration_ms {
        Some("moved".to_string())
    } else if before.voice_source_actual != after.voice_source_actual {
        Some(format!(
            "voice tier {} → {}",
            before.voice_source_actual, after.voice_source_actual
        ))
    } else {
        None
    }
}

pub fn diff(before: &NarrationManifest, after: &NarrationManifest) -> ManifestDiff {
    let added = after
        .segments
        .iter()
        .filter(|s| !before.segments.iter().any(|b| b.id == s.id))
        .map(|s| s.id.clone())
        .collect();
    let removed = before
        .segments
        .iter()
        .filter(|s| !after.segments.iter().any(|a| a.id == s.id))
        .map(|s| s.id.clone())
        .collect();

    let changed = before
        .segments
        .iter()
        .filter_map(|b| {
            let a = after.segments.iter().find(|a| a.id == b.id)?;
            reason_for(b, a).map(|reason| ChangedSegment {
                id: b.id.clone(),
                before_ms: b.duration_ms,
                after_ms: a.duration_ms,
                reason,
            })
        })
        .collect();

    // Order is part of the contract: a consumer iterating `segments` builds
    // its own sequence from them, so two identical segments that swapped
    // places are drift even though neither one changed.
    let before_order: Vec<&str> = before.segments.iter().map(|s| s.id.as_str()).collect();
    let after_order: Vec<&str> = after.segments.iter().map(|s| s.id.as_str()).collect();
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
        added,
        removed,
        changed,
        reordered: common_before != common_after,
    }
}
