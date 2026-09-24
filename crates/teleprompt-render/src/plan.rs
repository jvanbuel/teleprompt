//! Building a [`RenderPlan`] from the published narration manifest
//! (`docs/design.md#rendering`).

use std::path::PathBuf;

use teleprompt_compile::manifest::NarrationManifest;

use crate::{Narration, Picture, RenderPlan, Shot, Transition};

/// The scene name the manifest gives a shot that is only a pause.
const PAUSE: &str = "pause";

/// Where a render's inputs live, and the output's shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    /// The manifest's directory, which a line's `audio` is relative to.
    pub narration_dir: PathBuf,
    /// Captured clips, named by capture key.
    pub clips_dir: PathBuf,
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// Missing clips are warnings, not errors: they render as slates with the
/// real timing (`docs/design.md#rendering`).
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
        .shots
        .iter()
        .map(|shot| {
            // A pause holds, so it owes no clip. Clips are keyed by capture
            // key, not shot hash (`docs/design.md#capture-key`).
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
