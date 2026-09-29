//! Comparing two timelines and rendering the result as prose. Pure, and
//! iterates `BTreeMap`s keyed by item id, so output is byte-stable.

use std::collections::BTreeMap;
use teleprompt_core::config::TransitionKind;
use teleprompt_core::time::short;
use teleprompt_core::{DurationSource, ItemId};

use crate::timeline::{Entry, Timeline};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChangedBeat {
    pub item: ItemId,
    pub before_ms: u64,
    pub after_ms: u64,
    pub reason: ChangeReason,
}

/// Why a narrated item's timing changed, most actionable cause first.
/// Serialized as its prose, which `diff --format json` has always carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeReason {
    TextEdited,
    NowMeasured,
    AudioChanged,
    PaddingChanged,
}

impl std::fmt::Display for ChangeReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TextEdited => "text edited",
            Self::NowMeasured => "now measured",
            Self::AudioChanged => "audio changed",
            Self::PaddingChanged => "padding changed",
        })
    }
}

impl serde::Serialize for ChangeReason {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

/// An item on both sides that moved in the entry sequence. Indices are
/// 0-based positions in the full entry list; `render` prints them 1-based.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReorderedBeat {
    pub item: ItemId,
    pub before_index: usize,
    pub after_index: usize,
}

/// An item whose outgoing transition changed kind, length, or both.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChangedTransition {
    pub item: ItemId,
    pub before_kind: TransitionKind,
    pub after_kind: TransitionKind,
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
    pub recapture: Vec<String>,
}

impl TimelineDiff {
    /// True when nothing an author needs to look at changed. `shift_ms`
    /// counts too: edits such as retuning `output.transition.max_ms` move the
    /// total without changing any per-item fact, and `diff --exit-code` must
    /// still catch them.
    pub fn is_empty(&self) -> bool {
        self.shift_ms == 0
            && self.changed.is_empty()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.reordered.is_empty()
            && self.transitions.is_empty()
            && self.recapture.is_empty()
    }

    /// A prose report for a terminal or a PR comment; empty sections are
    /// omitted.
    pub fn render(&self) -> String {
        if self.is_empty() {
            return "no timeline changes".to_string();
        }

        // The printed shift is the difference of the rounded endpoints, not
        // the rounded `shift_ms`, so the header always adds up (1 ms → 50 ms
        // would otherwise print "0.0s → 0.1s (+0.0s)"). JSON keeps the
        // exact `shift_ms`.
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
                // A rewording can keep the same duration; "4.2s → 4.2s"
                // would read like a bug.
                if c.before_ms == c.after_ms {
                    out.push_str(&format!(
                        "\n  {:<16} {} (timing unchanged)  ({})",
                        c.item,
                        short(c.before_ms),
                        c.reason
                    ));
                } else {
                    out.push_str(&format!(
                        "\n  {:<16} {} \u{2192} {}  ({})",
                        c.item,
                        short(c.before_ms),
                        short(c.after_ms),
                        c.reason
                    ));
                }
            }
        }

        if !self.added.is_empty() {
            out.push_str("\n\nadded:");
            for item in &self.added {
                out.push_str(&format!("\n  {item}"));
            }
        }

        if !self.removed.is_empty() {
            out.push_str("\n\nremoved:");
            for item in &self.removed {
                out.push_str(&format!("\n  {item}"));
            }
        }

        if !self.reordered.is_empty() {
            out.push_str("\n\nreordered:");
            for r in &self.reordered {
                out.push_str(&format!(
                    "\n  {:<16} position {} \u{2192} {}",
                    r.item,
                    r.before_index + 1,
                    r.after_index + 1
                ));
            }
        }

        if !self.transitions.is_empty() {
            out.push_str("\n\ntransitions:");
            for t in &self.transitions {
                // Name the kind once unless it changed.
                if t.before_kind == t.after_kind {
                    out.push_str(&format!(
                        "\n  {:<16} {} {} \u{2192} {}",
                        t.item,
                        t.before_kind,
                        short(t.before_ms),
                        short(t.after_ms)
                    ));
                } else {
                    out.push_str(&format!(
                        "\n  {:<16} {} {} \u{2192} {} {}",
                        t.item,
                        t.before_kind,
                        short(t.before_ms),
                        t.after_kind,
                        short(t.after_ms)
                    ));
                }
            }
        }

        if !self.recapture.is_empty() {
            out.push_str("\n\nneeds recapture:");
            for item in &self.recapture {
                out.push_str(&format!("\n  {item}"));
            }
        }

        out
    }
}

/// Nearest tenth of a second, as an integer so two can be subtracted
/// exactly (8949 -> 89).
fn round_to_tenths(ms: u64) -> i64 {
    ((ms as f64) / 100.0).round() as i64
}

fn render_tenths(tenths: i64) -> String {
    format!("{:.1}s", tenths as f64 / 10.0)
}

fn render_signed_tenths(tenths: i64) -> String {
    let sign = if tenths >= 0 { "+" } else { "-" };
    format!("{sign}{:.1}s", tenths.unsigned_abs() as f64 / 10.0)
}

/// Compare two timelines, matching items by id.
pub fn diff(before: &Timeline, after: &Timeline) -> TimelineDiff {
    let old: BTreeMap<&str, &Entry> = before
        .entries
        .iter()
        .map(|e| (e.item.as_str(), e))
        .collect();
    let new: BTreeMap<&str, &Entry> = after.entries.iter().map(|e| (e.item.as_str(), e)).collect();

    let mut changed = Vec::new();
    let mut recapture = Vec::new();
    let mut transitions = Vec::new();

    for (id, n) in &new {
        if let Some(o) = old.get(id) {
            if o.transition != n.transition {
                transitions.push(ChangedTransition {
                    item: ItemId::from(*id),
                    before_kind: o.transition.kind.clone(),
                    after_kind: n.transition.kind.clone(),
                    before_ms: o.transition.duration_ms,
                    after_ms: n.transition.duration_ms,
                });
            }

            // Any hash or duration change counts, so a same-length
            // rewording still fails `--exit-code`. Placement is compared
            // relative to the item's start: absolute `start_ms` shifts with
            // every earlier edit and would bury the one that matters.
            if let (Some(on), Some(nn)) = (&o.narration, &n.narration) {
                let offset_moved = on.start_ms.saturating_sub(o.start_ms)
                    != nn.start_ms.saturating_sub(n.start_ms);
                // Only estimated -> measured: a cleared cache (the reverse)
                // changes nothing and must not read as "padding changed".
                let now_measured = on.duration_source != nn.duration_source
                    && nn.duration_source == DurationSource::Measured;
                if on.source_hash != nn.source_hash
                    || on.audio_hash != nn.audio_hash
                    || on.duration_ms != nn.duration_ms
                    || offset_moved
                    // Even when the measurement equals the estimate, as
                    // it always does with the `null` voice.
                    || now_measured
                {
                    let reason = if on.source_hash != nn.source_hash {
                        ChangeReason::TextEdited
                    // After `text edited`, which explains a duration change
                    // on its own; before the audio checks, which would
                    // otherwise report the wrong cause.
                    } else if now_measured {
                        ChangeReason::NowMeasured
                    } else if on.audio_hash != nn.audio_hash || on.duration_ms != nn.duration_ms {
                        ChangeReason::AudioChanged
                    } else {
                        ChangeReason::PaddingChanged
                    };
                    changed.push(ChangedBeat {
                        item: ItemId::from(*id),
                        before_ms: on.duration_ms,
                        after_ms: nn.duration_ms,
                        reason,
                    });
                }
            }

            // Recapture when an action appeared or vanished, or its capture
            // key, duration or offset within the item changed, or the item's
            // own length changed (the policy re-places the action). Only
            // local facts, so an item merely shifted by an earlier edit is
            // not flagged. Items with no action on either side are skipped.
            if o.action.is_some() || n.action.is_some() {
                let action_appeared_or_vanished = o.action.is_some() != n.action.is_some();
                let action_itself_changed = match (&o.action, &n.action) {
                    (Some(oa), Some(na)) => {
                        oa.capture_key != na.capture_key
                            || oa.duration_ms != na.duration_ms
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
        recapture,
    }
}

/// Items on both sides that play in a different order: those outside a
/// longest common subsequence, the smallest set whose removal makes the
/// orders agree. Swapping two paragraphs names one item, not every item
/// between them. Added and removed items are excluded so they are not
/// counted twice.
fn reordered(before: &Timeline, after: &Timeline) -> Vec<ReorderedBeat> {
    let in_after: std::collections::BTreeSet<&str> =
        after.entries.iter().map(|e| e.item.as_str()).collect();
    let in_before: std::collections::BTreeSet<&str> =
        before.entries.iter().map(|e| e.item.as_str()).collect();

    // Positions in the full list, matching what `teleprompt plan` shows.
    let old_pos: BTreeMap<&str, usize> = before
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.item.as_str(), i))
        .collect();
    let new_pos: BTreeMap<&str, usize> = after
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.item.as_str(), i))
        .collect();

    let a: Vec<&str> = before
        .entries
        .iter()
        .map(|e| e.item.as_str())
        .filter(|id| in_after.contains(id))
        .collect();
    let b: Vec<&str> = after
        .entries
        .iter()
        .map(|e| e.item.as_str())
        .filter(|id| in_before.contains(id))
        .collect();

    if a == b {
        return Vec::new();
    }

    let kept = longest_common_subsequence(&a, &b);
    b.iter()
        .filter(|id| !kept.contains(*id))
        .map(|id| ReorderedBeat {
            item: ItemId::from(*id),
            before_index: old_pos[id],
            after_index: new_pos[id],
        })
        .collect()
}

/// O(n·m) LCS, returned as a set because item ids are unique. Ties only
/// decide which of two swapped items is named, deterministically.
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
