//! The narration manifest: teleprompt's published output for pipelines that
//! render themselves.
//!
//! This is a **join**, not a projection. Timings come from the [`Timeline`],
//! but `text` and `chapters` live in the `Program` and `words` lives in the
//! `SynthResult` the scheduler discards. This crate is the only one holding
//! all three, which is why the type lives here rather than beside
//! `Timeline`.

use serde::{Deserialize, Serialize};
use teleprompt_core::program::ChapterInfo;
use teleprompt_core::Hash;
use teleprompt_schedule::Timeline;

use crate::NarrationDetail;

/// Incremented on any breaking change to the shape below. Independent of
/// `TIMELINE_VERSION`: the timeline is an internal review surface, the
/// manifest is a contract with third parties, and they will not move
/// together.
pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NarrationManifest {
    pub manifest_version: u32,
    pub script: String,
    pub locale: String,
    pub generated_by: String,
    pub duration_ms: u64,
    pub audio: AudioInfo,
    pub chapters: Vec<ChapterEntry>,
    pub segments: Vec<SegmentEntry>,
}

/// Uniform across every segment, so a consumer configures its player once
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentEntry {
    pub id: String,
    pub text: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub audio: String,
    pub voice_source: String,
    pub voice_source_actual: String,
    /// Always serialized. `null` when the requested tier was delivered —
    /// a consumer should not have to tell "absent" from "no downgrade".
    pub downgrade_reason: Option<String>,
    pub source_hash: Hash,
    pub audio_hash: Hash,
    /// Omitted entirely when the backend has no word timings, so absence is
    /// unambiguous and never confused with an empty list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<WordEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordEntry {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Where a segment's audio sits, relative to the manifest that names it.
pub fn audio_path(segment_id: &str, format: &str) -> String {
    format!("audio/{segment_id}.{format}")
}

/// Join a scheduled [`Timeline`] with the narration detail `compile`
/// retained, producing the published manifest.
///
/// A `NarrationDetail` with no matching timeline entry is dropped, and a
/// timeline narration entry with no matching detail is dropped: both are
/// impossible for output from a single `compile` call, and neither is worth
/// a fallible signature that every caller would then `unwrap`.
pub fn build(
    timeline: &Timeline,
    chapters: &[ChapterInfo],
    details: &[NarrationDetail],
    audio: AudioInfo,
) -> NarrationManifest {
    let segments: Vec<SegmentEntry> = timeline
        .entries
        .iter()
        .filter_map(|entry| {
            let n = entry.narration.as_ref()?;
            let detail = details.iter().find(|d| d.segment_id == n.segment)?;
            Some(SegmentEntry {
                id: n.segment.clone(),
                text: detail.text.clone(),
                start_ms: n.start_ms,
                duration_ms: n.duration_ms,
                audio: audio_path(&n.segment, &audio.format),
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

    // A chapter's start is its first spoken segment's start. A chapter with
    // nothing spoken in it has no defensible time and is omitted rather
    // than given a guessed one.
    let chapter_entries = chapters
        .iter()
        .filter_map(|c| {
            let start_ms = details
                .iter()
                .filter(|d| d.chapter == c.slug)
                .filter_map(|d| segments.iter().find(|s| s.id == d.segment_id))
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
        segments,
    }
}
