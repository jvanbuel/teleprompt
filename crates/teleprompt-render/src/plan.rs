//! Building a [`RenderPlan`] from the published narration manifest.
//!
//! `build` reads the same artifact an outside integrator reads, rather than
//! the in-process `Timeline` it could have kept in hand. Two timing paths
//! drift, and the one that drifts silently is the one nobody renders from:
//! if a render is ever a frame out, the manifest is wrong and the preview
//! is wrong with it.

use std::path::PathBuf;

use teleprompt_compile::manifest::NarrationManifest;

use crate::{Beat, Narration, Picture, RenderPlan, Transition};

/// The scene name the manifest gives a beat that is only a pause.
const PAUSE: &str = "pause";

/// Where the pieces of a render live, and what shape the result should be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    /// The directory holding the manifest. A segment's `audio` is relative
    /// to it.
    pub narration_dir: PathBuf,
    /// Where captured clips are looked up, by capture key.
    pub clips_dir: PathBuf,
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// The plan for `manifest`, and whatever a caller should be told about it.
///
/// Warnings rather than errors: a render with slates in it is a useful
/// thing to be able to watch — the timing is real and every offset is the
/// published one — and refusing to produce it until capture exists would
/// mean no video at all.
pub fn from_manifest(manifest: &NarrationManifest, inputs: &Inputs) -> (RenderPlan, Vec<String>) {
    let narration = manifest
        .segments
        .iter()
        .map(|segment| Narration {
            id: segment.id.clone(),
            path: inputs.narration_dir.join(&segment.audio),
            start_ms: segment.start_ms,
        })
        .collect();

    let mut uncaptured = 0usize;
    let beats: Vec<Beat> = manifest
        .beats
        .iter()
        .map(|beat| {
            // A pause is a beat during which the picture holds — that is
            // what a pause is. Looking for a clip under its hash would find
            // nothing and report a capture that was never owed.
            // Filed under the capture key, not the span hash: the same
            // tape in two places in a walkthrough is two pictures, because
            // the screen each one starts from is different.
            let clip = inputs.clips_dir.join(format!("{}.mp4", beat.capture_key));
            let picture = if beat.scene == PAUSE {
                Picture::Hold
            } else if clip.exists() {
                Picture::Clip(clip)
            } else {
                uncaptured += 1;
                Picture::Slate
            };
            Beat {
                id: beat.span.clone(),
                start_ms: beat.start_ms,
                duration_ms: beat.duration_ms,
                picture,
                transition: Transition {
                    kind: beat.transition.kind.clone(),
                    duration_ms: beat.transition.duration_ms,
                },
            }
        })
        .collect();

    let mut warnings = Vec::new();
    if uncaptured > 0 {
        warnings.push(format!(
            "{uncaptured} of {} beat(s) have no captured clip and will render as a slate; \
             nothing captures scenes yet, so the timing is real and the picture is not",
            beats.len()
        ));
    }

    (
        RenderPlan {
            width: inputs.width,
            height: inputs.height,
            fps: inputs.fps,
            duration_ms: manifest.duration_ms,
            beats,
            narration,
            output: inputs.output.clone(),
        },
        warnings,
    )
}
