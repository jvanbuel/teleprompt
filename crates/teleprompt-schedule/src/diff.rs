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

/// A beat that exists on both sides but has moved in the entry sequence.
/// Reordering is reported apart from `added`/`removed` because a reordered
/// beat is neither: nothing was written or deleted, the video just plays its
/// parts in a different order. Indices are 0-based positions in the full
/// entry list; `render` prints them 1-based.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReorderedBeat {
    pub beat: String,
    pub before_index: usize,
    pub after_index: usize,
}

/// A beat whose outgoing transition changed kind, length, or both.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChangedTransition {
    pub beat: String,
    pub before_kind: String,
    pub after_kind: String,
    pub before_ms: u64,
    pub after_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TimelineDiff {
    pub before_ms: u64,
    pub after_ms: u64,
    pub shift_ms: i64,
    pub changed: Vec<ChangedBeat>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub reordered: Vec<ReorderedBeat>,
    pub transitions: Vec<ChangedTransition>,
    pub stale_takes: Vec<StaleTake>,
    pub recapture: Vec<String>,
}

impl TimelineDiff {
    /// True when nothing an author needs to look at changed.
    ///
    /// Every field participates, `shift_ms` included. An earlier version of
    /// this method skipped the three duration fields on the grounds that they
    /// were "derived summaries of `changed`, not independent facts". They are
    /// not: the total is `last.start_ms + last.duration_ms`, and plenty of
    /// edits move it without touching any per-beat fact this diff inspects —
    /// retuning `output.transition.max_ms`, for one, changes every gap
    /// between beats while leaving each beat's own hashes and durations
    /// alone. Treating the total as derived is exactly what let
    /// `diff --exit-code` report clean over a genuinely stale committed
    /// timeline, which is the one thing spec §13 asks it to catch.
    pub fn is_empty(&self) -> bool {
        self.shift_ms == 0
            && self.changed.is_empty()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.reordered.is_empty()
            && self.transitions.is_empty()
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

        if !self.reordered.is_empty() {
            out.push_str("\n\nreordered:");
            for r in &self.reordered {
                out.push_str(&format!(
                    "\n  {:<16} position {} \u{2192} {}",
                    r.beat,
                    r.before_index + 1,
                    r.after_index + 1
                ));
            }
        }

        if !self.transitions.is_empty() {
            out.push_str("\n\ntransitions:");
            for t in &self.transitions {
                // Only name the kind twice when it actually changed; a
                // max_ms retune touches every beat's transition, and
                // repeating "crossfade → crossfade" on each line is noise.
                if t.before_kind == t.after_kind {
                    out.push_str(&format!(
                        "\n  {:<16} {} {} \u{2192} {}",
                        t.beat,
                        t.before_kind,
                        secs(t.before_ms),
                        secs(t.after_ms)
                    ));
                } else {
                    out.push_str(&format!(
                        "\n  {:<16} {} {} \u{2192} {} {}",
                        t.beat,
                        t.before_kind,
                        secs(t.before_ms),
                        t.after_kind,
                        secs(t.after_ms)
                    ));
                }
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
    let mut transitions = Vec::new();

    for (id, n) in &new {
        if let Some(o) = old.get(id) {
            // A beat's outgoing transition is part of the committed timeline
            // and is not implied by any other field: retuning
            // `output.transition.max_ms` re-times every gap while leaving
            // each beat's hashes and own duration untouched.
            if o.transition != n.transition {
                transitions.push(ChangedTransition {
                    beat: (*id).to_string(),
                    before_kind: o.transition.kind.clone(),
                    after_kind: n.transition.kind.clone(),
                    before_ms: o.transition.duration_ms,
                    after_ms: n.transition.duration_ms,
                });
            }

            // A narration change is reported whenever *any* of the three
            // narration facts differ, not only when duration does: a
            // reworded sentence of identical spoken length still leaves a
            // committed `source_hash` that disagrees with the script, and
            // a silent `diff` there is worse than a missing line — it lets
            // `--exit-code` report clean while the committed timeline is
            // stale. The two possible causes read very differently to the
            // author, so the reason still says which one happened.
            //
            // The narration's placement *within its own beat* counts too.
            // Its absolute `start_ms` deliberately does not: that shifts
            // whenever any earlier beat changes length, and flagging every
            // downstream beat would bury the one the author actually edited.
            // The offset from the beat's own start is local — it moves only
            // when this beat's padding or alignment moved — so it catches a
            // segment-level `lead_in=` retune that happens not to change the
            // beat's overall length, without any cascade.
            if let (Some(on), Some(nn)) = (&o.narration, &n.narration) {
                let offset_moved = on.start_ms.saturating_sub(o.start_ms)
                    != nn.start_ms.saturating_sub(n.start_ms);
                // Hoisted so the guard and the reason arm below cannot drift
                // apart. It has to be *this* condition and not a bare
                // `duration_source` inequality: the reverse transition
                // (measured -> estimated, i.e. a cleared cache) would then
                // enter the block and fall through to "padding changed",
                // which is a lie. A cleared cache is not a change to the
                // program, so it stays clean.
                let now_measured =
                    on.duration_source != nn.duration_source && nn.duration_source == "measured";
                if on.source_hash != nn.source_hash
                    || on.audio_hash != nn.audio_hash
                    || on.duration_ms != nn.duration_ms
                    || offset_moved
                    // A segment whose audio was synthesized since the last
                    // run reports as changed even when the measured length
                    // lands exactly on the estimate — which, with `null`, it
                    // always does. Without this the estimate-to-measurement
                    // transition is invisible to `teleprompt diff`, while
                    // `manifest_diff` reports it.
                    || now_measured
                {
                    let reason = if on.source_hash != nn.source_hash {
                        "text edited"
                    // Ordered after `text edited` deliberately: an author's
                    // edit is the cause they can act on, and it explains the
                    // duration change by itself. Ordered before the audio and
                    // duration checks because those would otherwise absorb
                    // this and report a cause that sends the reader to the
                    // wrong place.
                    } else if now_measured {
                        "now measured"
                    } else if on.audio_hash != nn.audio_hash || on.duration_ms != nn.duration_ms {
                        "audio changed"
                    } else {
                        "padding changed"
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
                        oa.span_hash != na.span_hash
                            || oa.duration_ms != na.duration_ms
                            // Same local-offset reasoning as narration above:
                            // where in its beat the action sits is a fact of
                            // this beat alone, so comparing it costs no
                            // cascade noise and catches an alignment change
                            // that leaves the beat's length intact.
                            || oa.start_ms.saturating_sub(o.start_ms)
                                != na.start_ms.saturating_sub(n.start_ms)
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
        reordered: reordered(before, after),
        transitions,
        stale_takes,
        recapture,
    }
}

/// Beats that survive the edit but play in a different order.
///
/// Only beats present on both sides take part: an added or removed beat
/// necessarily shifts everything after it, and reporting those shifts as
/// "reordered" would double-count a change already named under
/// `added`/`removed`. What is left is compared as two sequences, and the
/// beats reported are those *outside* a longest common subsequence of the
/// two — the smallest set whose removal makes the orders agree. Swapping two
/// paragraphs therefore names one beat, and moving a chapter's opening beat
/// to the end names that beat rather than every beat it passed.
fn reordered(before: &Timeline, after: &Timeline) -> Vec<ReorderedBeat> {
    let in_after: std::collections::BTreeSet<&str> =
        after.entries.iter().map(|e| e.beat.as_str()).collect();
    let in_before: std::collections::BTreeSet<&str> =
        before.entries.iter().map(|e| e.beat.as_str()).collect();

    // Position in the *full* entry list, so the rendered positions match what
    // an author counts in `teleprompt plan`.
    let old_pos: BTreeMap<&str, usize> = before
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.beat.as_str(), i))
        .collect();
    let new_pos: BTreeMap<&str, usize> = after
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.beat.as_str(), i))
        .collect();

    let a: Vec<&str> = before
        .entries
        .iter()
        .map(|e| e.beat.as_str())
        .filter(|id| in_after.contains(id))
        .collect();
    let b: Vec<&str> = after
        .entries
        .iter()
        .map(|e| e.beat.as_str())
        .filter(|id| in_before.contains(id))
        .collect();

    if a == b {
        return Vec::new();
    }

    let kept = longest_common_subsequence(&a, &b);
    b.iter()
        .filter(|id| !kept.contains(*id))
        .map(|id| ReorderedBeat {
            beat: (*id).to_string(),
            before_index: old_pos[id],
            after_index: new_pos[id],
        })
        .collect()
}

/// Standard O(n·m) LCS over two id sequences, returned as a set. Beat ids are
/// unique within a timeline, so membership is all the caller needs. Ties in
/// the DP resolve toward `a`, which only decides *which* of two mutually
/// swapped beats is named as having moved — deterministically either way.
fn longest_common_subsequence<'a>(
    a: &[&'a str],
    b: &[&'a str],
) -> std::collections::BTreeSet<&'a str> {
    let (n, m) = (a.len(), b.len());
    let mut table = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if a[i] == b[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }

    let mut kept = std::collections::BTreeSet::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            kept.insert(a[i]);
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    kept
}
