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
///
/// **2** added `beats`. A consumer could place speech and nothing else, so
/// anything wanting to show a picture had to read the timeline and join it
/// to the manifest by document order — the exact fragility a published
/// contract exists to remove. Adding a key is backward compatible for a
/// consumer that ignores unknown fields, but silently missing the beats is
/// worse than failing: a v1 consumer that checks the version now stops and
/// says so.
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
    pub segments: Vec<SegmentEntry>,
    /// One entry per scheduled action span, in document order.
    ///
    /// `segments` says when each sentence is spoken; without this, that is
    /// all a consumer knows, and every pacing decision the scheduler made —
    /// `hold`, `concurrent`, `fit-action`, `trim-action` — stops at the
    /// boundary. The numbers here are the ones the scheduler arrived at,
    /// not the ones the adapter proposed, which is the difference between
    /// replaying a tape at its authored pace and replaying it at the pace
    /// its narration bought.
    pub beats: Vec<BeatEntry>,
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

/// One scheduled action span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeatEntry {
    /// The span's id, `<block>#<index>`, as the adapter minted it.
    pub span: String,
    /// The segment spoken over this span, or `null`.
    ///
    /// Always serialized. A span is paired with narration only when it is
    /// the first span of the block that followed a paragraph; every span
    /// after a mark runs under whatever the policy left of that paragraph,
    /// and a pause has no narration at all. A consumer should not have to
    /// tell "absent" from "unpaired".
    pub segment: Option<String>,
    pub scene: String,
    pub adapter: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    /// `exact` when the adapter's language states the span's timing in full,
    /// `measured` from a measuring pass, `estimated` from a model. Same
    /// vocabulary as a segment's, applied to an action.
    pub duration_source: String,
    pub policy: String,
    /// The outgoing transition, as scheduled. Carried because a consumer
    /// reproducing the video's shape needs the gaps between beats, and
    /// deriving them from neighbouring offsets is exactly the arithmetic
    /// that goes wrong where beats overlap.
    pub transition: TransitionOut,
    /// Identity of the span's source, for a per-span render cache. The
    /// counterpart of a segment's `audio_hash`.
    pub span_hash: Hash,
    /// Identity of the *picture*: this span's source and every span before
    /// it in the same session. What a captured clip is filed under.
    ///
    /// Not the same as `span_hash`, and the difference is the point. A
    /// scene is a session, so the screen a beat shows is the accumulation
    /// of every beat before it; two blocks with the same steps — `j` twice
    /// in one walkthrough — have one `span_hash` and two pictures.
    pub capture_key: Hash,
    /// Which run of the scene, from `session="…"`. Published because it is
    /// the only part of the chain a reader cannot see in the script, and
    /// because a capture stage needs it to know which beats share a screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionOut {
    pub kind: String,
    pub duration_ms: u64,
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
    /// [`build`] cannot fill this in: it runs before anything is encoded.
    /// The only hash it has in hand is the `Timeline`'s `audio_hash`, which
    /// is a deliberately *different* quantity — the identity of the audio a
    /// segment resolves to, derived from its synthesis cache key, which is
    /// why it does not move when `dub` re-renders byte-identical audio.
    /// This field describes the file: the bytes, and nothing else. `build`
    /// seeds it with the timeline's value and `teleprompt dub` overwrites
    /// it with `Hash::of(&wav_bytes)` after encoding. See [`build`]'s note.
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
/// audio, so each segment's `audio_hash` is set to the `Timeline`'s value —
/// which answers a different question: *which* audio this segment resolves
/// to, rather than what the file on disk contains. `teleprompt dub`
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

    // One beat per scheduled span, in the timeline's own order. A pause is
    // included rather than filtered: `scene: "pause"` is a beat during which
    // the picture holds, and a consumer that skipped it would run the next
    // span early.
    let beats: Vec<BeatEntry> = timeline
        .entries
        .iter()
        .filter_map(|entry| {
            let a = entry.action.as_ref()?;
            Some(BeatEntry {
                span: a.span.clone(),
                segment: entry.narration.as_ref().map(|n| n.segment.clone()),
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
                span_hash: a.span_hash,
                capture_key: a.capture_key,
                session: a.session.clone(),
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
        beats,
    }
}
