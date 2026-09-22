//! Drift detection for a committed narration manifest.
//!
//! This is what keeps teleprompt's central claim working when the picture
//! is rendered by something teleprompt does not control: a pull request
//! that edits prose without regenerating fails, exactly as one with a
//! stale timeline does.

use serde::Serialize;

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
    /// `AudioInfo` (format, sample rate, channels) differs. A consumer's
    /// player is configured from this once, uniformly, so a change here is
    /// real drift even when every line's own fields are untouched.
    pub audio_changed: bool,
    /// `chapters` differs. A title edit that slugifies identically — "Quick
    /// start" → "Quick Start" — changes nothing in any `LineEntry`, so
    /// this has to be its own flag or that edit is invisible to `diff`.
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
            secs(self.duration_before_ms),
            secs(self.duration_after_ms),
            if delta >= 0 { "+" } else { "-" },
            secs(delta.unsigned_abs()),
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
                    secs(c.before_ms),
                    secs(c.after_ms),
                    c.reason
                ));
            }
        }
        if self.reordered {
            out.push_str("\nreordered: line order changed\n");
        }

        // A line that only `"shifted"` has byte-identical audio — it
        // landed at a new `start_ms` because an earlier line's length
        // changed, not because anything about this line did. Listing it
        // here would ask for a re-render nothing needs and bury the one
        // line the author actually touched. See `reason_for` below.
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

fn secs(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

/// Why a line differs, in the order that makes the report most useful:
/// a text edit is the author's own doing and explains everything
/// downstream, so it is named even when the audio and position also moved.
///
/// `duration_ms` changing is reported as `"audio changed"`, not
/// `"shifted"`: nothing moved when only this line's own length changed.
/// `"shifted"` is reserved for the case where `start_ms` is the *only*
/// thing that differs — that line's audio is byte-identical, it simply
/// landed somewhere else because an earlier line's length changed
/// upstream of it. `teleprompt-schedule/src/diff.rs:288-291` makes the same
/// call for the timeline's own narration diff, deliberately excluding a
/// beat's absolute `start_ms` from what triggers a report there, for the
/// same reason: flagging every downstream beat would bury the one the
/// author actually edited. The manifest must not contradict its sibling.
fn reason_for(before: &LineEntry, after: &LineEntry) -> Option<String> {
    if before.source_hash != after.source_hash {
        Some("text edited".to_string())
    // Ordered exactly as the timeline's own narration diff orders it
    // (`teleprompt-schedule/src/diff.rs`): after `text edited`, because an
    // author's edit is the cause they can act on and explains the duration
    // change by itself; before the audio and duration checks, which would
    // otherwise absorb this and send the reader looking for a content
    // change that did not happen. The manifest must not contradict its
    // sibling.
    //
    // The reverse transition — measured back to estimated, a cleared cache
    // — deliberately does not claim something was just measured.
    //
    // Inert while `dub`'s recompile means every published manifest says
    // `measured`. Without it the field carries no information at all, and
    // with `null`, where the estimate and the render always coincide,
    // `--check` would call an estimated-to-measured transition clean.
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

    // Order is part of the contract: a consumer iterating `lines` builds
    // its own sequence from them, so two identical lines that swapped
    // places are drift even though neither one changed.
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
