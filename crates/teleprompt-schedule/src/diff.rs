//! Comparing two committed timelines and rendering the result as prose.
//!
//! `diff` is a pure function: no I/O, no clock, no randomness. Given the
//! same two `Timeline`s it always returns the same `TimelineDiff`, and
//! iteration is always over `BTreeMap`s keyed by beat id so both the
//! returned data and its rendered/JSON forms are byte-identical across
//! runs.

use std::collections::BTreeMap;

use crate::timeline::{Entry, Timeline};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChangedBeat {
    pub beat: String,
    pub before_ms: u64,
    pub after_ms: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StaleTake {
    pub segment: String,
    pub falls_back_to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TimelineDiff {
    pub before_ms: u64,
    pub after_ms: u64,
    pub shift_ms: i64,
    pub changed: Vec<ChangedBeat>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub stale_takes: Vec<StaleTake>,
    pub recapture: Vec<String>,
}

impl TimelineDiff {
    /// True when nothing an author needs to look at changed. Note this
    /// deliberately ignores `shift_ms`/`before_ms`/`after_ms`: those are
    /// derived summaries of `changed`, not independent facts, so a zero
    /// shift with no changed beats and no added/removed/stale/recapture
    /// entries is the only way to get here.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.stale_takes.is_empty()
            && self.recapture.is_empty()
    }

    /// Render the diff as a short prose report, suitable for a terminal or
    /// a PR comment. Sections are omitted when empty, so a diff that only
    /// touched narration timing doesn't show blank "added"/"removed"
    /// headers.
    pub fn render(&self) -> String {
        if self.is_empty() {
            return "no timeline changes".to_string();
        }

        let mut out = format!(
            "timeline: {} \u{2192} {} ({})",
            secs(self.before_ms),
            secs(self.after_ms),
            signed(self.shift_ms)
        );

        if !self.changed.is_empty() {
            out.push_str("\n\nchanged:");
            for c in &self.changed {
                out.push_str(&format!(
                    "\n  {:<16} {} \u{2192} {}  ({})",
                    c.beat,
                    secs(c.before_ms),
                    secs(c.after_ms),
                    c.reason
                ));
            }
        }

        if !self.added.is_empty() {
            out.push_str("\n\nadded:");
            for beat in &self.added {
                out.push_str(&format!("\n  {beat}"));
            }
        }

        if !self.removed.is_empty() {
            out.push_str("\n\nremoved:");
            for beat in &self.removed {
                out.push_str(&format!("\n  {beat}"));
            }
        }

        if !self.stale_takes.is_empty() {
            out.push_str("\n\nstale takes (build will fall back unless re-recorded):");
            for s in &self.stale_takes {
                out.push_str(&format!(
                    "\n  {:<16} falls back to {}",
                    s.segment, s.falls_back_to
                ));
            }
        }

        if !self.recapture.is_empty() {
            out.push_str("\n\nneeds recapture:");
            for beat in &self.recapture {
                out.push_str(&format!("\n  {beat}"));
            }
        }

        out
    }
}

fn secs(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

fn signed(ms: i64) -> String {
    let sign = if ms >= 0 { "+" } else { "-" };
    format!("{sign}{:.1}s", ms.unsigned_abs() as f64 / 1000.0)
}

/// Compare two committed timelines. Beats are matched by id; anything not
/// present in `before` is `added`, anything not present in `after` is
/// `removed`. Everything else below only runs for beats present in both.
pub fn diff(before: &Timeline, after: &Timeline) -> TimelineDiff {
    let old: BTreeMap<&str, &Entry> = before
        .entries
        .iter()
        .map(|e| (e.beat.as_str(), e))
        .collect();
    let new: BTreeMap<&str, &Entry> = after.entries.iter().map(|e| (e.beat.as_str(), e)).collect();

    let mut changed = Vec::new();
    let mut recapture = Vec::new();
    let mut stale_takes = Vec::new();

    for (id, n) in &new {
        if let Some(o) = old.get(id) {
            // Narration duration is the thing that ripples through the
            // whole timeline: a longer clip pushes every later beat's
            // start_ms out. The two possible causes read very differently
            // to the author, so the reason must say which one happened,
            // not just that something changed.
            if let (Some(on), Some(nn)) = (&o.narration, &n.narration) {
                if on.duration_ms != nn.duration_ms {
                    let reason = if on.source_hash != nn.source_hash {
                        "text edited"
                    } else {
                        "audio changed"
                    };
                    changed.push(ChangedBeat {
                        beat: (*id).to_string(),
                        before_ms: on.duration_ms,
                        after_ms: nn.duration_ms,
                        reason: reason.to_string(),
                    });
                }
            }

            // A beat needs recapture when its steps changed (span_hash),
            // when its rendered action duration changed (e.g. a stretch/
            // trim policy re-timed it), or when the beat's own local
            // duration changed — which happens when narration got longer
            // or shorter even though the action's steps are identical,
            // because Hold/Concurrent/etc. reposition the action relative
            // to narration within the beat. `duration_ms` here is each
            // beat's own local duration, not the timeline total, so a beat
            // whose *position* shifted only because an earlier, unrelated
            // beat changed does NOT get flagged — only a beat whose own
            // internal layout actually moved does.
            if let (Some(oa), Some(na)) = (&o.action, &n.action) {
                if oa.span_hash != na.span_hash
                    || oa.duration_ms != na.duration_ms
                    || o.duration_ms != n.duration_ms
                {
                    recapture.push((*id).to_string());
                }
            }
        }

        // Stale takes are a property of the new timeline alone: whatever
        // voice source was requested no longer matches what actually got
        // used, regardless of whether anything else changed this diff.
        if let Some(nn) = &n.narration {
            if nn.voice_source != nn.voice_source_actual {
                stale_takes.push(StaleTake {
                    segment: nn.segment.clone(),
                    falls_back_to: nn.voice_source_actual.clone(),
                });
            }
        }
    }

    let added: Vec<String> = new
        .keys()
        .filter(|id| !old.contains_key(*id))
        .map(|id| (*id).to_string())
        .collect();
    let removed: Vec<String> = old
        .keys()
        .filter(|id| !new.contains_key(*id))
        .map(|id| (*id).to_string())
        .collect();

    TimelineDiff {
        before_ms: before.duration_ms,
        after_ms: after.duration_ms,
        shift_ms: after.duration_ms as i64 - before.duration_ms as i64,
        changed,
        added,
        removed,
        stale_takes,
        recapture,
    }
}
