//! Building the narration manifest. It is a join, not a projection: timings
//! come from the [`Timeline`], while text, chapters and word timings come
//! from what `compile` kept in [`NarrationDetail`]. Only this crate holds
//! both.

use crate::schedule::Timeline;
use teleprompt_manifest::{
    audio_path, AudioInfo, ChapterEntry, LineEntry, NarrationManifest, ShotEntry, WordEntry,
    MANIFEST_VERSION,
};
use teleprompt_script::program::ChapterInfo;

use crate::compile::NarrationDetail;

/// Joins a scheduled [`Timeline`] with the [`NarrationDetail`] `compile`
/// kept. Unmatched entries on either side are dropped; a single `compile`
/// call cannot produce them.
///
/// `audio_hash` is seeded with the timeline's value, which identifies the
/// audio a line resolves to, not the file's bytes. Nothing here has
/// encoded audio, so `dub` overwrites it with the hash of what it wrote,
/// and callers that never synthesize need not invent one.
pub fn build(
    timeline: &Timeline,
    chapters: &[ChapterInfo],
    details: &[NarrationDetail],
    audio: AudioInfo,
) -> NarrationManifest {
    let lines: Vec<LineEntry> = timeline
        .entries
        .iter()
        .filter_map(|entry| {
            let n = entry.narration.as_ref()?;
            let detail = details.iter().find(|d| d.line_id == n.line)?;
            Some(LineEntry {
                id: n.line.clone(),
                text: detail.text.clone(),
                chapter: detail.chapter.clone(),
                start_ms: n.start_ms,
                duration_ms: n.duration_ms,
                duration_source: n.duration_source,
                audio: audio_path(&n.line, &audio.format),
                source_hash: n.source_hash,
                audio_hash: n.audio_hash,
                words: detail.word_timings.as_ref().map(|ws| {
                    // At the tempo the line is played at.
                    let at = |ms: u64| {
                        n.tempo_permille
                            .map_or(ms, |t| ms * 1000 / u64::from(t.permille()))
                    };
                    ws.iter()
                        .map(|w| WordEntry {
                            text: w.word.clone(),
                            start_ms: at(w.start_ms),
                            end_ms: at(w.end_ms),
                        })
                        .collect()
                }),
                tempo_permille: n.tempo_permille,
                speaker: detail.speaker.clone(),
                name: detail.name.clone(),
            })
        })
        .collect();

    // Pauses stay in: a consumer that skipped one would run the next shot
    // early.
    let shots: Vec<ShotEntry> = timeline
        .entries
        .iter()
        .filter_map(|entry| {
            let a = entry.action.as_ref()?;
            Some(ShotEntry {
                shot: a.shot.clone(),
                line: entry.narration.as_ref().map(|n| n.line.clone()),
                scene: a.scene.clone(),
                plugin: a.plugin.clone(),
                start_ms: a.start_ms,
                duration_ms: a.duration_ms,
                duration_source: a.duration_source,
                policy: entry.policy,
                transition: teleprompt_core::config::Transition {
                    kind: entry.transition.kind.clone(),
                    duration_ms: entry.transition.duration_ms,
                },
                shot_hash: a.shot_hash,
                capture_key: a.capture_key,
                session: a.session.clone(),
            })
        })
        .collect();

    // A chapter starts at its first spoken line; one with nothing spoken
    // has no defensible time and is omitted. The join is by index, not by
    // slug: slugs are not deduplicated, so two `# Setup` chapters share one.
    let chapter_entries = chapters
        .iter()
        .enumerate()
        .filter_map(|(index, c)| {
            let start_ms = details
                .iter()
                .filter(|d| d.chapter_index == index)
                .filter_map(|d| lines.iter().find(|s| s.id == d.line_id))
                .map(|s| s.start_ms)
                .min()?;
            Some(ChapterEntry {
                id: c.slug.clone(),
                title: c.title.clone(),
                start_ms,
            })
        })
        .collect();

    NarrationManifest {
        manifest_version: MANIFEST_VERSION,
        script: timeline.script.clone(),
        locale: timeline.locale.clone(),
        generated_by: timeline.generated_by.clone(),
        duration_ms: timeline.duration_ms,
        audio,
        chapters: chapter_entries,
        lines,
        shots,
    }
}
