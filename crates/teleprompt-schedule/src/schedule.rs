use teleprompt_core::config::TransitionDuration;

use crate::item::{DurationSource, Item};
use crate::policy::layout_at;
use crate::timeline::{ActionEntry, Entry, NarrationEntry, Timeline, TransitionEntry};

pub const TIMELINE_VERSION: u32 = 1;

/// Pure. Returns the timeline and its warnings, each prefixed with its item.
pub fn schedule(
    items: &[Item],
    script: &str,
    locale: &str,
    version: &str,
) -> (Timeline, Vec<String>) {
    let mut entries: Vec<Entry> = Vec::with_capacity(items.len());
    let mut warnings = Vec::new();
    let mut cursor = 0u64;

    // Time the incoming transition already took from this item; a
    // transition overlaps both items it joins.
    let mut carried_in = 0u64;

    for (i, item) in items.iter().enumerate() {
        let timing = &item.config.timing;

        // Padding comes from the narration (see `NarrationInput::lead_in_ms`);
        // `timing` is the action block's, which is right for the action's
        // stretch and speedup bounds.
        let narration_ms = item
            .narration
            .as_ref()
            .map(|n| n.padded_duration_ms())
            .unwrap_or(0);
        // An `Unknown` action takes the length of its narration.
        let action_ms = item
            .action
            .as_ref()
            .map(|a| match a.duration_source {
                DurationSource::Unknown => narration_ms,
                _ => a.duration_ms,
            })
            .unwrap_or(0);

        // A cue counts from the first word, which comes after the lead-in.
        let lead_in_ms = item.narration.as_ref().map_or(0, |n| n.lead_in_ms);
        let cue_ms = item
            .action
            .as_ref()
            .and_then(|a| a.cue_ms)
            .map(|c| c + lead_in_ms);
        let l = layout_at(item.policy, narration_ms, action_ms, cue_ms, timing);
        for w in &l.warnings {
            warnings.push(format!("{}: {w}", item.id));
        }

        // The previous item's outgoing transition was capped against that
        // item only; if this item is shorter, give back the excess so two
        // blends never draw the same frames.
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

        // docs/design.md#quiet-window: this item's trailing silence plus the
        // next item's lead-in.
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
                // Honoured as written, with a warning if it overlaps speech.
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
                // The quiet-window cap overrides `min_ms` on purpose.
                TransitionDuration::Auto => (slack / 2)
                    .clamp(item.config.transition.min_ms, item.config.transition.max_ms)
                    .min(quiet_window_ms),
            }
        }
        // Never more than the item has left after its incoming transition;
        // otherwise an item could fit each transition but not both, and the
        // plan could not be cut into chunks.
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
                    DurationSource::Unknown => "unknown",
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
                // Placeholder: `compile` chains the real key after re-timing,
                // from the source that will actually be captured.
                capture_key: a.shot_hash,
                session: a.session.clone(),
                start_ms: cursor + l.action_start_ms,
                duration_ms: l.action_duration_ms,
                duration_source: match a.duration_source {
                    DurationSource::Exact => "exact",
                    DurationSource::Estimated => "estimated",
                    DurationSource::Measured => "measured",
                    DurationSource::Unknown => "unknown",
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
