use teleprompt_core::config::TransitionDuration;

use teleprompt_core::DurationSource;

use crate::item::{ActionInput, Item, NarrationInput};
use crate::policy::{layout_at, Layout};
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
        let (narration_ms, l) = lay_out_item(item);
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
            cursor = cursor.saturating_add(excess);
            carried_in = l.item_duration_ms;
        }

        let transition_ms = match items.get(i + 1) {
            None => 0,
            Some(next) => {
                let slack = narration_ms.saturating_sub(l.action_duration_ms);
                let quiet_window_ms = quiet_window_ms(item, next, &l);
                size_transition(item, slack, quiet_window_ms, &mut warnings)
            }
        }
        // Never more than the item has left after its incoming transition;
        // otherwise an item could fit each transition but not both, and the
        // plan could not be cut into chunks.
        .min(l.item_duration_ms.saturating_sub(carried_in));

        entries.push(build_entry(item, cursor, &l, transition_ms));
        carried_in = transition_ms;
        cursor = cursor.saturating_add(l.item_duration_ms - transition_ms);
    }

    let duration_ms = entries
        .last()
        .map(|e| e.start_ms.saturating_add(e.duration_ms))
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

/// Lays out one item under its policy. Returns the padded narration length
/// alongside the layout, since transition slack is measured against it.
fn lay_out_item(item: &Item) -> (u64, Layout) {
    // Padding comes from the narration (see `NarrationInput::lead_in_ms`);
    // `item.pacing.timing` is the action block's, which is right for the
    // action's stretch and speedup bounds.
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
    let lead_in_ms = item.narration.as_ref().map_or(0, |n| n.lead_in_ms.ms());
    let cue_ms = item
        .action
        .as_ref()
        .and_then(|a| a.cue_ms)
        .map(|c| c.saturating_add(lead_in_ms));
    let l = layout_at(
        item.policy,
        narration_ms,
        action_ms,
        cue_ms,
        &item.pacing.timing,
    );
    (narration_ms, l)
}

/// docs/design.md#quiet-window: this item's trailing silence plus the next
/// item's lead-in.
fn quiet_window_ms(item: &Item, next: &Item, l: &Layout) -> u64 {
    let speech_end_ms = item
        .narration
        .as_ref()
        .map(|n| {
            l.narration_start_ms
                .saturating_add(n.lead_in_ms.ms())
                .saturating_add(n.duration_ms)
        })
        .unwrap_or(0);
    let next_lead_in_ms = next.narration.as_ref().map_or(0, |n| n.lead_in_ms.ms());
    l.item_duration_ms
        .saturating_sub(speech_end_ms)
        .saturating_add(next_lead_in_ms)
}

/// The configured duration of the transition out of `item`, before it is
/// capped by what the item has left.
fn size_transition(
    item: &Item,
    slack_ms: u64,
    quiet_window_ms: u64,
    warnings: &mut Vec<String>,
) -> u64 {
    let transition = &item.pacing.transition;
    match transition.duration {
        // Honoured as written, with a warning if it overlaps speech.
        TransitionDuration::Fixed(ms) => {
            let ms = ms.ms();
            if ms > quiet_window_ms {
                warnings.push(format!(
                    "{}: fixed {kind} of {ms}ms exceeds the {quiet_window_ms}ms quiet \
                     window; narration will overlap by {}ms",
                    item.id,
                    ms - quiet_window_ms,
                    kind = transition.kind,
                ));
            }
            ms
        }
        // The quiet-window cap overrides `min_ms` on purpose.
        TransitionDuration::Auto => (slack_ms / 2)
            .clamp(transition.min_ms.ms(), transition.max_ms.ms())
            .min(quiet_window_ms),
    }
}

/// The timeline entry for an item laid out as `l` and starting at `start_ms`.
fn build_entry(item: &Item, start_ms: u64, l: &Layout, transition_ms: u64) -> Entry {
    Entry {
        item: item.id.clone(),
        start_ms,
        duration_ms: l.item_duration_ms,
        policy: item.policy.kind(),
        narration: item
            .narration
            .as_ref()
            .map(|n| narration_entry(n, start_ms.saturating_add(l.narration_start_ms))),
        action: item.action.as_ref().map(|a| action_entry(a, start_ms, l)),
        transition: TransitionEntry {
            kind: item.pacing.transition.kind.clone(),
            duration_ms: transition_ms,
        },
    }
}

/// `slot_start_ms` is where the padded line begins; the entry records the
/// first word, after the lead-in.
fn narration_entry(n: &NarrationInput, slot_start_ms: u64) -> NarrationEntry {
    NarrationEntry {
        line: n.line_id.clone(),
        source_hash: n.source_hash,
        audio_hash: n.audio_hash,
        start_ms: slot_start_ms.saturating_add(n.lead_in_ms.ms()),
        duration_ms: n.duration_ms,
        duration_source: n.duration_source,
        recorded: n.recorded,
    }
}

fn action_entry(a: &ActionInput, item_start_ms: u64, l: &Layout) -> ActionEntry {
    ActionEntry {
        shot: a.shot_id.clone(),
        scene: a.scene.clone(),
        adapter: a.adapter.clone(),
        shot_hash: a.shot_hash,
        // Placeholder: `compile` chains the real key after re-timing,
        // from the source that will actually be captured.
        capture_key: a.shot_hash,
        session: a.session.clone(),
        start_ms: item_start_ms.saturating_add(l.action_start_ms),
        duration_ms: l.action_duration_ms,
        duration_source: a.duration_source,
    }
}
