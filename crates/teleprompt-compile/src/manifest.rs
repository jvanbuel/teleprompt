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
    /// Slug of the chapter this segment was spoken in. Always serialized,
    /// including when that chapter has no entry in `chapters` — a chapter
    /// is only omitted there when nothing in it is spoken, which cannot be
    /// true of a chapter that owns a segment, but a consumer must not have
    /// to reason about that to answer "which chapter is this?".
    ///
    /// Published because the alternative is reconstruction from timestamps
    /// against a `chapters` list that deliberately omits silent chapters,
    /// which is lossy.
    pub chapter: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    /// `measured` when this came from real audio, `estimated` when it is the
    /// duration model's prediction. A consumer building a player can show the
    /// difference; one committing the manifest should know a re-dub will move
    /// every estimated segment.
    pub duration_source: String,
    pub audio: String,
    pub voice_source: String,
    pub voice_source_actual: String,
    /// Always serialized. `null` when the requested tier was delivered —
    /// a consumer should not have to tell "absent" from "no downgrade".
    pub downgrade_reason: Option<String>,
    pub source_hash: Hash,
    /// Hash of the **encoded audio file** named by `audio`, so a consumer
    /// can cache renders and skip re-encoding.
    ///
    /// [`build`] cannot fill this in: it runs before anything is encoded,
    /// and the only hash it has in hand is the `Timeline`'s, which is the
    /// backend's *synthesis cache key* — a different thing that embeds the
    /// teleprompt version and so changes on every release. `build` seeds
    /// the field with that key and `teleprompt dub` overwrites it with
    /// `Hash::of(&wav_bytes)` after encoding. See [`build`]'s note.
    ///
    /// **Repeated values are not a bug.** This hashes content, so audio
    /// that is byte-identical hashes identically — which is the whole
    /// point, since that is exactly when a consumer may reuse a cached
    /// render. It is conspicuous with the `null` backend in particular:
    /// `null` emits pure silence, so every segment of the same duration
    /// produces the same bytes and therefore the same hash, and a manifest
    /// full of repeated hashes is the correct output. Real speech differs
    /// per segment and the repetition disappears. Segment identity for
    /// drift purposes comes from `source_hash`, which does not collide.
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
///
/// **`audio_hash` is seeded, not final.** Nothing here has encoded any
/// audio, so each segment's `audio_hash` is set to the `Timeline`'s value,
/// which is the voice backend's synthesis cache key. `teleprompt dub`
/// replaces it with the hash of the bytes it actually wrote. The
/// alternative — taking a `&[(String, Hash)]` of byte hashes here — was
/// rejected because it would force every non-rendering caller (`build`'s
/// own tests, and any future consumer that wants the manifest shape
/// without paying for synthesis) to invent hashes for audio it never made.
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
                chapter: detail.chapter.clone(),
                start_ms: n.start_ms,
                duration_ms: n.duration_ms,
                duration_source: n.duration_source.clone(),
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
    //
    // The join is **positional**, not by slug. Slugs derive from titles
    // (`teleprompt_core::ast::slugify`), cannot be pinned, and are never
    // deduplicated, so two `# Setup` chapters share the slug `setup`.
    // Joining on it made `min()` run across both chapters' segments: the
    // two markers came out with the same `start_ms` and the second
    // chapter's real start was lost. `ChapterEntry.id` stays the slug —
    // that is what a consumer wants to see — but the join behind it is the
    // index.
    let chapter_entries = chapters
        .iter()
        .enumerate()
        .filter_map(|(index, c)| {
            let start_ms = details
                .iter()
                .filter(|d| d.chapter_index == index)
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
