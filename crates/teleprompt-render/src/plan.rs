//! Building a [`RenderPlan`] from the published narration manifest.
//!
//! `build` reads the same artifact an outside integrator reads, rather than
//! the in-process `Timeline` it could have kept in hand. Two timing paths
//! drift, and the one that drifts silently is the one nobody renders from:
//! if a render is ever a frame out, the manifest is wrong and the preview
//! is wrong with it.

use std::path::PathBuf;

use teleprompt_compile::manifest::NarrationManifest;

use crate::{Narration, Picture, RenderPlan, Shot, Transition};

/// The scene name the manifest gives a shot that is only a pause.
const PAUSE: &str = "pause";

/// Where the placements of a render live, and what shape the result should be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    /// The directory holding the manifest. A line's `audio` is relative
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
/// published one — and refusing to produce it because one machine has no
/// backend for one scene would mean no video at all.
pub fn from_manifest(manifest: &NarrationManifest, inputs: &Inputs) -> (RenderPlan, Vec<String>) {
    let narration = manifest
        .lines
        .iter()
        .map(|line| Narration {
            id: line.id.clone(),
            path: inputs.narration_dir.join(&line.audio),
            start_ms: line.start_ms,
        })
        .collect();

    let mut uncaptured = 0usize;
    let shots: Vec<Shot> = manifest
        .items
        .iter()
        .map(|shot| {
            // A pause is a shot during which the picture holds — that is
            // what a pause is. Looking for a clip under its hash would find
            // nothing and report a capture that was never owed.
            // Filed under the capture key, not the shot hash: the same
            // tape in two places in a walkthrough is two pictures, because
            // the screen each one starts from is different.
            let clip = inputs.clips_dir.join(format!("{}.mp4", shot.capture_key));
            let picture = if shot.scene == PAUSE {
                Picture::Hold
            } else if clip.exists() {
                Picture::Clip(clip)
            } else {
                uncaptured += 1;
                Picture::Slate
            };
            Shot {
                id: shot.shot.clone(),
                start_ms: shot.start_ms,
                duration_ms: shot.duration_ms,
                picture,
                transition: Transition {
                    kind: shot.transition.kind.clone(),
                    duration_ms: shot.transition.duration_ms,
                },
            }
        })
        .collect();

    let mut warnings = Vec::new();
    if uncaptured > 0 {
        warnings.push(format!(
            "{uncaptured} of {} shot(s) have no captured clip and will render as a slate; \
             the timing is real and the picture is not",
            shots.len()
        ));
    }

    (
        RenderPlan {
            width: inputs.width,
            height: inputs.height,
            fps: inputs.fps,
            duration_ms: manifest.duration_ms,
            shots,
            narration,
            output: inputs.output.clone(),
        },
        warnings,
    )
}
