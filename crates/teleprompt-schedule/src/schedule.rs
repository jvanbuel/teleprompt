use teleprompt_core::config::TransitionDuration;

use crate::beat::{Beat, DurationSource};
use crate::policy::layout;
use crate::timeline::{ActionEntry, Entry, NarrationEntry, Timeline, TransitionEntry};

pub const TIMELINE_VERSION: u32 = 1;

/// Pure: same inputs always produce the same timeline.
/// Returns the timeline plus any policy warnings, tagged with their beat.
pub fn schedule(
    beats: &[Beat],
    script: &str,
    locale: &str,
    version: &str,
) -> (Timeline, Vec<String>) {
    let mut entries: Vec<Entry> = Vec::with_capacity(beats.len());
    let mut warnings = Vec::new();
    let mut cursor = 0u64;

    for (i, beat) in beats.iter().enumerate() {
        let timing = &beat.config.timing;

        // Padding comes from the narration itself, not from `beat.config`:
        // the beat's config is the action block's layer, and a segment's own
        // `lead_in=`/`tail=` attributes would otherwise be discarded whenever
        // an action block followed the paragraph. `timing` below is still the
        // action block's, which is correct — `max_stretch`/`max_speedup`
        // govern the action.
        let narration_ms = beat
            .narration
            .as_ref()
            .map(|n| n.padded_duration_ms())
            .unwrap_or(0);
        let action_ms = beat.action.as_ref().map(|a| a.duration_ms).unwrap_or(0);

        let l = layout(beat.policy, narration_ms, action_ms, timing);
        for w in &l.warnings {
            warnings.push(format!("{}: {w}", beat.id));
        }

        let slack = narration_ms.saturating_sub(l.action_duration_ms);
        let is_last = i + 1 == beats.len();
        let transition_ms = if is_last {
            0
        } else {
            match beat.config.transition.duration {
                TransitionDuration::Fixed(ms) => ms,
                TransitionDuration::Auto => {
                    (slack / 2).clamp(beat.config.transition.min_ms, beat.config.transition.max_ms)
                }
            }
        }
        // A transition can never consume more than the beat it leaves.
        .min(l.beat_duration_ms);

        entries.push(Entry {
            beat: beat.id.clone(),
            start_ms: cursor,
            duration_ms: l.beat_duration_ms,
            policy: beat.policy.label().to_string(),
            narration: beat.narration.as_ref().map(|n| NarrationEntry {
                segment: n.segment_id.clone(),
                source_hash: n.source_hash,
                audio_hash: n.audio_hash,
                start_ms: cursor + l.narration_start_ms + n.lead_in_ms,
                duration_ms: n.duration_ms,
                voice_source: n.voice_source.label().to_string(),
                voice_source_actual: n.voice_source_actual.label().to_string(),
                downgrade_reason: n.downgrade_reason.clone(),
            }),
            action: beat.action.as_ref().map(|a| ActionEntry {
                span: a.span_id.clone(),
                scene: a.scene.clone(),
                adapter: a.adapter.clone(),
                span_hash: a.span_hash,
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
                kind: beat.config.transition.kind.clone(),
                duration_ms: transition_ms,
            },
        });

        cursor += l.beat_duration_ms - transition_ms;
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
