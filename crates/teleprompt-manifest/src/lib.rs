//! The narration manifest: the published contract a renderer reads
//! (docs/design.md#manifest). `teleprompt-compile` builds it; renderers,
//! the preview and outside consumers read it.

use serde::{Deserialize, Serialize};
use teleprompt_core::{DurationSource, Hash, PolicyKind, VoiceSource};

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
    pub duration_source: DurationSource,
    pub policy: PolicyKind,
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
    pub duration_source: DurationSource,
    pub audio: String,
    pub voice_source: VoiceSource,
    pub voice_source_actual: VoiceSource,
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

pub mod diff;
