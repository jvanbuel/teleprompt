use teleprompt_core::config::TransitionDuration;

use crate::item::{DurationSource, Item};
use crate::policy::layout_at;
use crate::timeline::{ActionEntry, Entry, NarrationEntry, Timeline, TransitionEntry};

pub const TIMELINE_VERSION: u32 = 1;

/// Pure: same inputs always produce the same timeline.
/// Returns the timeline plus any policy warnings, tagged with their item.
pub fn schedule(
    items: &[Item],
    script: &str,
    locale: &str,
    version: &str,
) -> (Timeline, Vec<String>) {
    let mut entries: Vec<Entry> = Vec::with_capacity(items.len());
    let mut warnings = Vec::new();
    let mut cursor = 0u64;

    // How much of this item the transition arriving into it already took.
    // A transition overlaps *both* items it joins, so it is charged to each.
    let mut carried_in = 0u64;

    for (i, item) in items.iter().enumerate() {
        let timing = &item.config.timing;

        // Padding comes from the narration itself, not from `item.config`:
        // the item's config is the action block's layer, and a line's own
        // `lead_in=`/`tail=` attributes would otherwise be discarded whenever
        // an action block followed the paragraph. `timing` below is still the
        // action block's, which is correct — `max_stretch`/`max_speedup`
        // govern the action.
        let narration_ms = item
            .narration
            .as_ref()
            .map(|n| n.padded_duration_ms())
            .unwrap_or(0);
        let action_ms = item.action.as_ref().map(|a| a.duration_ms).unwrap_or(0);

        let cue_ms = item.action.as_ref().and_then(|a| a.cue_ms);
        let l = layout_at(item.policy, narration_ms, action_ms, cue_ms, timing);
        for w in &l.warnings {
            warnings.push(format!("{}: {w}", item.id));
        }

        // The transition granted on the way out of the previous item was
        // capped against *that* item. It also eats into this one, which may
        // be shorter than the transition is long. Give the time back before
        // this item is placed, rather than letting two blends draw the same
        // frames twice.
        if carried_in > l.item_duration_ms {
            let excess = carried_in - l.item_duration_ms;
            if let Some(previous) = entries.last_mut() {
                previous.transition.duration_ms -= excess;
            }
            cursor += excess;
            carried_in = l.item_duration_ms;
        }

        let slack = narration_ms.saturating_sub(l.action_duration_ms);
        let is_last = i + 1 == items.len();

        // Spec §6.3, the quiet window: a transition overlaps the item it
        // leaves, so it may only consume time when nobody is speaking. That is
        // this item's trailing silence — from where its narration ends to
        // where the item ends, normally `tail` — plus the next item's
        // `lead_in`.
        //
        // Without this, a script of plain paragraphs derives `slack` from the
        // whole narration, because there is no action to subtract, and every
        // transition runs to `max_ms` and eats real speech.
        let speech_end_ms = item
            .narration
            .as_ref()
            .map(|n| l.narration_start_ms + n.lead_in_ms + n.duration_ms)
            .unwrap_or(0);
        let next_lead_in_ms = items
            .get(i + 1)
            .and_then(|b| b.narration.as_ref())
            .map(|n| n.lead_in_ms)
            .unwrap_or(0);
        let quiet_window_ms = l.item_duration_ms.saturating_sub(speech_end_ms) + next_lead_in_ms;

        let transition_ms = if is_last {
            0
        } else {
            match item.config.transition.duration {
                // Honoured as written: the author asked for this length by
                // name. But a fixed transition wider than the quiet window
                // does produce two voices at once, so say so.
                TransitionDuration::Fixed(ms) => {
                    if ms > quiet_window_ms {
                        warnings.push(format!(
                            "{}: fixed {kind} of {ms}ms exceeds the {quiet_window_ms}ms quiet \
                             window; narration will overlap by {}ms",
                            item.id,
                            ms - quiet_window_ms,
                            kind = item.config.transition.kind,
                        ));
                    }
                    ms
                }
                // The cap deliberately overrides `min_ms`: a shorter
                // transition than configured, or none at all, is better than
                // one that talks over the narration.
                TransitionDuration::Auto => (slack / 2)
                    .clamp(item.config.transition.min_ms, item.config.transition.max_ms)
                    .min(quiet_window_ms),
            }
        }
        // A transition can never consume more than the item it leaves, nor
        // more than that item has left after the transition that arrived
        // into it. Without the second cap an item can be long enough for
        // each transition alone and too short for both together — which is
        // a plan that cannot be cut into chunks at all.
        .min(l.item_duration_ms.saturating_sub(carried_in));

        entries.push(Entry {
            item: item.id.clone(),
            start_ms: cursor,
            duration_ms: l.item_duration_ms,
            policy: item.policy.label().to_string(),
            narration: item.narration.as_ref().map(|n| NarrationEntry {
                line: n.line_id.clone(),
                source_hash: n.source_hash,
                audio_hash: n.audio_hash,
                start_ms: cursor + l.narration_start_ms + n.lead_in_ms,
                duration_ms: n.duration_ms,
                duration_source: match n.duration_source {
                    DurationSource::Exact => "exact",
                    DurationSource::Estimated => "estimated",
                    DurationSource::Measured => "measured",
                }
                .to_string(),
                voice_source: n.voice_source.label().to_string(),
                voice_source_actual: n.voice_source_actual.label().to_string(),
                downgrade_reason: n.downgrade_reason.clone(),
            }),
            action: item.action.as_ref().map(|a| ActionEntry {
                shot: a.shot_id.clone(),
                scene: a.scene.clone(),
                adapter: a.adapter.clone(),
                shot_hash: a.shot_hash,
                // Filled by `compile`'s chaining pass, which runs after
                // re-timing: a shot that was re-written to fit its slot is
                // a different tape, and the chain has to be built from the
                // tape that will actually be captured.
                capture_key: a.shot_hash,
                session: a.session.clone(),
                start_ms: cursor + l.action_start_ms,
                duration_ms: l.action_duration_ms,
                duration_source: match a.duration_source {
                    DurationSource::Exact => "exact",
                    DurationSource::Estimated => "estimated",
                    DurationSource::Measured => "measured",
                }
                .to_string(),
            }),
            transition: TransitionEntry {
                kind: item.config.transition.kind.clone(),
                duration_ms: transition_ms,
            },
        });

        carried_in = transition_ms;
        cursor += l.item_duration_ms - transition_ms;
    }

    let duration_ms = entries
        .last()
        .map(|e| e.start_ms + e.duration_ms)
        .unwrap_or(0);

    (
        Timeline {
            version: TIMELINE_VERSION,
            script: script.to_string(),
            locale: locale.to_string(),
            duration_ms,
            generated_by: format!("teleprompt {version}"),
            entries,
        },
        warnings,
    )
}
