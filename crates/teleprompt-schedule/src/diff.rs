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

        // The header's three numbers must reconcile: before + shift == after,
        // as printed. Rounding `before_ms`, `after_ms`, and the raw
        // millisecond `shift_ms` independently to one decimal is not
        // guaranteed to agree, because rounding to tenths isn't linear —
        // e.g. before_ms=1 rounds to 0.0s and after_ms=50 rounds to 0.1s,
        // but the exact 49ms difference between them rounds to 0.0s on its
        // own, which would print "0.0s → 0.1s (+0.0s)": a header that
        // visibly doesn't add up. So the displayed shift is derived from
        // the two already-rounded endpoints instead of from the raw delta:
        // round both endpoints to tenths of a second first, then subtract
        // those integers. `self.shift_ms` itself stays exact in the struct
        // and in JSON for machine consumers; only this prose line is
        // affected.
        let before_tenths = round_to_tenths(self.before_ms);
        let after_tenths = round_to_tenths(self.after_ms);
        let shift_tenths = after_tenths - before_tenths;

        let mut out = format!(
            "timeline: {} \u{2192} {} ({})",
            render_tenths(before_tenths),
            render_tenths(after_tenths),
            render_signed_tenths(shift_tenths)
        );

        if !self.changed.is_empty() {
            out.push_str("\n\nchanged:");
            for c in &self.changed {
                // A text edit doesn't have to move the clock: two phrasings
                // can land on the same spoken duration. Writing
                // "4.2s → 4.2s" there reads like a rendering bug, so an
                // unchanged duration gets its own phrasing that still says
                // plainly that this beat changed.
                if c.before_ms == c.after_ms {
                    out.push_str(&format!(
                        "\n  {:<16} {} (timing unchanged)  ({})",
                        c.beat,
                        secs(c.before_ms),
                        c.reason
                    ));
                } else {
                    out.push_str(&format!(
                        "\n  {:<16} {} \u{2192} {}  ({})",
                        c.beat,
                        secs(c.before_ms),
                        secs(c.after_ms),
                        c.reason
                    ));
                }
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

/// Rounds a millisecond count to the nearest tenth of a second, returned as
/// an integer count of tenths (e.g. 8949 -> 89, 15499 -> 155). Kept as an
/// integer rather than a float so two roundings can be subtracted exactly,
/// with no risk of the subtraction itself reintroducing float rounding
/// error.
fn round_to_tenths(ms: u64) -> i64 {
    ((ms as f64) / 100.0).round() as i64
}

/// Renders an already-rounded tenths-of-a-second count (see
/// `round_to_tenths`) back out as `"12.3s"`.
fn render_tenths(tenths: i64) -> String {
    format!("{:.1}s", tenths as f64 / 10.0)
}

/// Renders an already-rounded, signed tenths-of-a-second count as
/// `"+12.3s"` / `"-4.5s"`.
fn render_signed_tenths(tenths: i64) -> String {
    let sign = if tenths >= 0 { "+" } else { "-" };
    format!("{sign}{:.1}s", tenths.unsigned_abs() as f64 / 10.0)
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
            // A narration change is reported whenever *any* of the three
            // narration facts differ, not only when duration does: a
            // reworded sentence of identical spoken length still leaves a
            // committed `source_hash` that disagrees with the script, and
            // a silent `diff` there is worse than a missing line — it lets
            // `--exit-code` report clean while the committed timeline is
            // stale. The two possible causes read very differently to the
            // author, so the reason still says which one happened.
            if let (Some(on), Some(nn)) = (&o.narration, &n.narration) {
                if on.source_hash != nn.source_hash
                    || on.audio_hash != nn.audio_hash
                    || on.duration_ms != nn.duration_ms
                {
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
            // trim policy re-timed it), when the beat's own local duration
            // changed — which happens when narration got longer or shorter
            // even though the action's steps are identical, because Hold/
            // Concurrent/etc. reposition the action relative to narration
            // within the beat — or when an action was added or removed
            // outright. `duration_ms` here is each beat's own local
            // duration, not the timeline total, so a beat whose *position*
            // shifted only because an earlier, unrelated beat changed does
            // NOT get flagged — only a beat whose own internal layout
            // actually moved does.
            //
            // The duration comparison runs whenever *either* side carries
            // an action — it is not gated on both sides carrying one, so a
            // beat that gains or loses its action is still caught by it as
            // well as by the explicit presence check below. A beat with no
            // action on either side has nothing to recapture, so it is
            // skipped entirely rather than flagged on narration timing
            // alone.
            if o.action.is_some() || n.action.is_some() {
                let action_appeared_or_vanished = o.action.is_some() != n.action.is_some();
                let action_itself_changed = match (&o.action, &n.action) {
                    (Some(oa), Some(na)) => {
                        oa.span_hash != na.span_hash || oa.duration_ms != na.duration_ms
                    }
                    _ => false,
                };
                if action_appeared_or_vanished
                    || action_itself_changed
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
