use teleprompt_core::config::TransitionDuration;

use crate::beat::{Beat, DurationSource};
use crate::policy::layout_at;
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
        // the beat's config is the action block's layer, and a line's own
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

        let at_ms = beat.action.as_ref().and_then(|a| a.at_ms);
        let l = layout_at(beat.policy, narration_ms, action_ms, at_ms, timing);
        for w in &l.warnings {
            warnings.push(format!("{}: {w}", beat.id));
        }

        let slack = narration_ms.saturating_sub(l.action_duration_ms);
        let is_last = i + 1 == beats.len();

        // Spec §6.3, the quiet window: a transition overlaps the beat it
        // leaves, so it may only consume time when nobody is speaking. That is
        // this beat's trailing silence — from where its narration ends to
        // where the beat ends, normally `tail` — plus the next beat's
        // `lead_in`.
        //
        // Without this, a script of plain paragraphs derives `slack` from the
        // whole narration, because there is no action to subtract, and every
        // transition runs to `max_ms` and eats real speech.
        let speech_end_ms = beat
            .narration
            .as_ref()
            .map(|n| l.narration_start_ms + n.lead_in_ms + n.duration_ms)
            .unwrap_or(0);
        let next_lead_in_ms = beats
            .get(i + 1)
            .and_then(|b| b.narration.as_ref())
            .map(|n| n.lead_in_ms)
            .unwrap_or(0);
        let quiet_window_ms = l.beat_duration_ms.saturating_sub(speech_end_ms) + next_lead_in_ms;

        let transition_ms = if is_last {
            0
        } else {
            match beat.config.transition.duration {
                // Honoured as written: the author asked for this length by
                // name. But a fixed transition wider than the quiet window
                // does produce two voices at once, so say so.
                TransitionDuration::Fixed(ms) => {
                    if ms > quiet_window_ms {
                        warnings.push(format!(
                            "{}: fixed {kind} of {ms}ms exceeds the {quiet_window_ms}ms quiet \
                             window; narration will overlap by {}ms",
                            beat.id,
                            ms - quiet_window_ms,
                            kind = beat.config.transition.kind,
                        ));
                    }
                    ms
                }
                // The cap deliberately overrides `min_ms`: a shorter
                // transition than configured, or none at all, is better than
                // one that talks over the narration.
                TransitionDuration::Auto => (slack / 2)
                    .clamp(beat.config.transition.min_ms, beat.config.transition.max_ms)
                    .min(quiet_window_ms),
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
            action: beat.action.as_ref().map(|a| ActionEntry {
                cue: a.span_id.clone(),
                scene: a.scene.clone(),
                adapter: a.adapter.clone(),
                cue_hash: a.cue_hash,
                // Filled by `compile`'s chaining pass, which runs after
                // re-timing: a cue that was re-written to fit its slot is
                // a different tape, and the chain has to be built from the
                // tape that will actually be captured.
                capture_key: a.cue_hash,
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
