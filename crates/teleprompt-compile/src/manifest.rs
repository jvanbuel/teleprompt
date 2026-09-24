//! The narration manifest (docs/design.md#manifest).
//!
//! It is a join, not a projection: timings come from the [`Timeline`], while
//! text, chapters and word timings come from what `compile` kept in
//! [`NarrationDetail`]. Only this crate holds both.

use serde::{Deserialize, Serialize};
use teleprompt_core::program::ChapterInfo;
use teleprompt_core::Hash;
use teleprompt_schedule::Timeline;

use crate::NarrationDetail;

/// Incremented on any change a consumer must not silently miss, including
/// an added key it would otherwise ignore. Independent of
/// `TIMELINE_VERSION`: the manifest is a third-party contract, the timeline
/// an internal review surface.
pub const MANIFEST_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NarrationManifest {
    pub manifest_version: u32,
    pub script: String,
    pub locale: String,
    pub generated_by: String,
    pub duration_ms: u64,
    pub audio: AudioInfo,
    pub chapters: Vec<ChapterEntry>,
    pub lines: Vec<LineEntry>,
    /// One entry per scheduled action shot, in document order, with the
    /// timings the scheduler arrived at rather than the adapter's own.
    pub shots: Vec<ShotEntry>,
}

/// Uniform across every line, so a consumer configures its player once
/// rather than probing each file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioInfo {
    pub format: String,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterEntry {
    pub id: String,
    pub title: String,
    pub start_ms: u64,
}

/// One scheduled action shot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShotEntry {
    /// The shot's id, `<block>#<index>`, as the adapter minted it.
    pub shot: String,
    /// The line paired with this shot, or `null` (always serialized). Only
    /// the first shot of a block that follows a paragraph is paired.
    pub line: Option<String>,
    pub scene: String,
    pub adapter: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    /// `exact` when the source states its timing in full, `estimated` for a
    /// bound, `unknown` when the adapter cannot say and the shot took its
    /// line's length.
    pub duration_source: String,
    pub policy: String,
    /// The outgoing transition, as scheduled. Deriving it from neighbouring
    /// offsets goes wrong where items overlap.
    pub transition: TransitionOut,
    /// Identity of the shot's source.
    pub shot_hash: Hash,
    /// Identity of the picture, chained over the session
    /// (docs/design.md#capture-key). Two identical shots share a
    /// `shot_hash` but not a `capture_key`.
    pub capture_key: Hash,
    /// Which run of the scene, from `session="…"`: tells a capture stage
    /// which shots share a screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionOut {
    pub kind: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineEntry {
    pub id: String,
    pub text: String,
    /// Slug of the chapter this line was spoken in, so a consumer need not
    /// reconstruct it from timestamps.
    pub chapter: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    /// `measured` from real audio, `estimated` from the estimator
    /// (docs/design.md#estimated-and-measured).
    pub duration_source: String,
    pub audio: String,
    pub voice_source: String,
    pub voice_source_actual: String,
    /// `null` when the requested tier was delivered (always serialized).
    pub downgrade_reason: Option<String>,
    pub source_hash: Hash,
    /// Hash of the audio file's bytes, set by `dub` after encoding; see
    /// [`build`]. Byte-identical audio repeats a hash (every same-length
    /// line of `null` silence does), and that is correct: line identity is
    /// `source_hash`.
    pub audio_hash: Hash,
    /// Omitted when the backend gave no word timings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<WordEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordEntry {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Where a line's audio sits, relative to the manifest that names it.
pub fn audio_path(line_id: &str, format: &str) -> String {
    format!("audio/{line_id}.{format}")
}

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
                duration_source: n.duration_source.clone(),
                audio: audio_path(&n.line, &audio.format),
                voice_source: n.voice_source.clone(),
                voice_source_actual: n.voice_source_actual.clone(),
                downgrade_reason: n.downgrade_reason.clone(),
                source_hash: n.source_hash,
                audio_hash: n.audio_hash,
                words: detail.word_timings.as_ref().map(|ws| {
                    ws.iter()
                        .map(|w| WordEntry {
                            text: w.word.clone(),
                            start_ms: w.start_ms,
                            end_ms: w.end_ms,
                        })
                        .collect()
                }),
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
                adapter: a.adapter.clone(),
                start_ms: a.start_ms,
                duration_ms: a.duration_ms,
                duration_source: a.duration_source.clone(),
                policy: entry.policy.clone(),
                transition: TransitionOut {
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
